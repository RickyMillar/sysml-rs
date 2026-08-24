//! # sysml-mcp — MCP Server for SysML v2
//!
//! Exposes the [`SysmlService`] API as MCP (Model Context Protocol) tools,
//! allowing AI agents to load, query, analyze, and visualize SysML v2 models.
//!
//! ## Transport
//!
//! Communicates over stdio using JSON-RPC (the standard MCP transport).
//! All logging goes to stderr; stdout is reserved for protocol messages.

// rmcp re-exports schemars 1.x which the derive macro needs in scope.
// The workspace uses schemars 0.8 — we must NOT import it here.
use rmcp::schemars;
use rmcp::tool;

use std::collections::HashMap;
use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, ServerInfo};
use rmcp::service::NotificationContext;
use rmcp::ServiceExt;
use serde::Deserialize;
use sysml_service::SysmlService;

/// MCP server handler wrapping the SysML service.
pub struct SysmlMcpHandler {
    pub service: Arc<SysmlService>,
    tool_router: ToolRouter<Self>,
    /// Guard ensuring the progress forwarder is spawned at most once per
    /// handler. The first `on_initialized` call captures the peer and
    /// spawns the forwarder; later calls (re-init handshakes) are a no-op.
    progress_forwarder_spawned: std::sync::atomic::AtomicBool,
}

impl SysmlMcpHandler {
    /// Create a new handler with the given service.
    pub fn new(service: Arc<SysmlService>) -> Self {
        Self {
            service,
            tool_router: Self::tool_router(),
            progress_forwarder_spawned: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

// ---------------------------------------------------------------------------
// Request types — use rmcp::schemars (v1) for JsonSchema derive
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LoadSourceRequest {
    /// Identifier for the source (used as lookup key)
    pub uri: String,
    /// SysML v2 source text to parse and load
    pub source: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UriRequest {
    /// URI of the loaded model
    pub uri: String,
}

/// Bucket B / B4: multi-URI model tree across every loaded user file.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WorkspaceModelTreeRequest {
    /// Maximum tree depth to recurse (omit for unlimited).
    pub max_depth: Option<usize>,
    /// Tree view: "full" (default — every element kind) or "user_facing"
    /// (mirrors the simulation UI projection).
    pub view: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ModelTreeRequest {
    /// URI of the loaded model
    pub uri: String,
    /// Maximum tree depth to return (omit for unlimited). At the cutoff depth,
    /// children are replaced with a single summary node showing the count of
    /// hidden children.
    pub max_depth: Option<usize>,
    /// Which subset of elements to project.
    ///
    /// - `"full"` (default for MCP / AI-agent callers): every element kind is
    ///   present — ports for physics coupling, FeatureTyping bindings for
    ///   refactoring, expression AST sub-nodes, transitions, the lot. This
    ///   is the right choice for code-style inspection or analysis.
    /// - `"user_facing"`: mirrors what the simulation UI sees — drops
    ///   spec-mandated wrappers (memberships, FeatureTyping, expression AST),
    ///   currently-hidden domain kinds (ports, flows, connections,
    ///   transitions), and chrome (comments, imports). Use this if you want
    ///   to see a model the way a human exploring the Run page would.
    ///
    /// Anything else (omitted, typo) → `"full"` for MCP callers.
    pub view: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryRequest {
    /// URI of the loaded model to query; use "__workspace__" for merged workspace queries
    pub uri: String,
    /// Structured QuerySpec JSON. Omit `limit` to use MCP's context-safe default of 100 rows.
    pub spec: serde_json::Value,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RequirementRowsRequest {
    /// RequirementRowsSpec JSON: { filter?, limit?, cursor? }. Omit for defaults
    /// (all rows, MCP default page size).
    pub spec: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FindRequest {
    /// URI of the loaded model to search
    pub uri: String,
    /// Name pattern to search for (substring match)
    pub pattern: String,
    /// Optional element kind filter (e.g. "PartUsage", "RequirementUsage")
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ElementRequest {
    /// URI of the loaded model
    pub uri: String,
    /// The element's unique identifier
    pub id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FilePathRequest {
    /// Absolute or relative path to a `.sysml` file on disk
    pub path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UnloadFileRequest {
    /// URI of the loaded model to remove (the same key returned by load_file/load_source)
    pub uri: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WorkspaceRequest {
    /// Absolute or relative path to the workspace root directory
    pub path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConstraintCheckRequest {
    /// URI of the loaded model to check constraints against
    pub uri: String,
    /// Optional key-value overrides applied to the evaluation context (e.g. {"mass": "120", "max_mass": "100"})
    pub overrides: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExpressionEvalRequest {
    /// SysML expression to evaluate (e.g. "mass + 10", "speed > 100")
    pub expression: String,
    /// Optional key-value context bindings for variables used in the expression
    pub context: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TraceMatrixRequest {
    /// URI of the loaded model
    pub uri: String,
    /// Source element kind (e.g. "RequirementUsage", "PartUsage")
    pub source_kind: String,
    /// Target element kind (e.g. "PartUsage", "ActionUsage")
    pub target_kind: String,
    /// Relationship kind connecting source to target (e.g. "satisfy", "verify", "derive", "allocate")
    pub relation_kind: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SimulateStartRequest {
    /// URI of the loaded model containing the state machine
    pub uri: String,
    /// Name of the state machine definition to simulate
    pub sm_name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SimulateStepRequest {
    /// Session key returned by sysml_simulate_start (format: "uri:sm_name")
    pub session_key: String,
    /// Optional event/trigger name to inject (omit for autonomous step)
    pub event: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SessionKeyRequest {
    /// Session key returned by the corresponding start operation
    pub session_key: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SessionIdRequest {
    /// Opaque UUID returned by any session-starting command
    pub session_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ActionStartRequest {
    /// URI of the loaded model containing the action definition
    pub uri: String,
    /// Name of the action definition to execute
    pub action_name: String,
}

/// Generic JSON parameters for pass-through dispatch to service commands.
///
/// Used by MCP tools that directly delegate to the service registry
/// without needing typed request structs.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct JsonParams {
    /// Arbitrary parameters forwarded to the service command
    #[serde(flatten)]
    pub params: HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

fn ok_json<T: serde::Serialize>(value: &T) -> Result<CallToolResult, rmcp::ErrorData> {
    match serde_json::to_string_pretty(value) {
        Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
        Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
            "serialization error: {e}"
        ))])),
    }
}

fn ok_json_value(value: &serde_json::Value) -> Result<CallToolResult, rmcp::ErrorData> {
    match serde_json::to_string_pretty(value) {
        Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
        Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
            "serialization error: {e}"
        ))])),
    }
}

fn err_result(msg: impl std::fmt::Display) -> Result<CallToolResult, rmcp::ErrorData> {
    Ok(CallToolResult::error(vec![Content::text(msg.to_string())]))
}

/// Dispatch a command to the service layer via the inventory-based command registry.
///
/// This is the adapter bridge: MCP tool handlers can delegate here for commands
/// that map 1:1 to a `#[service_command]`-annotated method.
fn dispatch_to_service(
    service: &SysmlService,
    command_name: &str,
    params: serde_json::Value,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match sysml_service::execute_command(service, command_name, params) {
        Ok(result) => ok_json_value(&result),
        Err(e) => err_result(e),
    }
}

/// Like [`dispatch_to_service`] but appends a `_readiness` field to the
/// JSON response envelope, derived from `SysmlService::readiness_for(uri)`.
///
/// Option B from P-RA5: existing tool responses are unchanged; only the
/// top-level JSON object grows a single additive field. When the
/// underlying response is not an object (array / scalar), the response
/// is wrapped as `{ "result": <body>, "_readiness": <readiness> }` so
/// the field has a home. Wire compatibility for legacy callers that
/// ignore unknown fields is preserved.
///
/// `uri` is the URI the readiness snapshot should be taken against —
/// typically the same URI the tool is operating on.
fn dispatch_to_service_with_readiness(
    service: &SysmlService,
    command_name: &str,
    params: serde_json::Value,
    uri: &str,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match sysml_service::execute_command(service, command_name, params) {
        Ok(result) => {
            let readiness = service.readiness_for(uri);
            let readiness_json = match serde_json::to_value(&readiness) {
                Ok(v) => v,
                Err(e) => {
                    return err_result(format!("readiness serialization error: {e}"));
                }
            };
            let envelope = match result {
                serde_json::Value::Object(mut map) => {
                    // Field name is `_readiness` (leading underscore) so it
                    // does not collide with any field a service command
                    // already returns and so callers that filter on
                    // tool-specific keys still see the original shape.
                    map.insert("_readiness".to_owned(), readiness_json);
                    serde_json::Value::Object(map)
                }
                other => serde_json::json!({
                    "result": other,
                    "_readiness": readiness_json,
                }),
            };
            ok_json_value(&envelope)
        }
        Err(e) => {
            // Even on error, surface readiness so the client can reason
            // about "still loading, retry later" vs "permanent failure".
            let readiness = service.readiness_for(uri);
            let readiness_json = serde_json::to_value(&readiness).unwrap_or(serde_json::Value::Null);
            let payload = serde_json::json!({
                "error": e.to_string(),
                "_readiness": readiness_json,
            });
            match serde_json::to_string_pretty(&payload) {
                Ok(s) => Ok(CallToolResult::error(vec![Content::text(s)])),
                Err(_) => err_result(e),
            }
        }
    }
}

#[rmcp::tool_router]
impl SysmlMcpHandler {
    /// Load SysML source text directly (no file I/O). Returns the URI key.
    #[tool(name = "sysml_load_source")]
    async fn load_source(
        &self,
        params: Parameters<LoadSourceRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        dispatch_to_service(&self.service, "sysml.load_source", serde_json::json!({
            "uri": req.uri,
            "source": req.source,
        }))
    }

    /// Unload a previously-loaded URI from the analysis host. Returns true if the URI was loaded (and is now unloaded), false if it was already absent.
    #[tool(name = "sysml_unload_file")]
    async fn unload_file(
        &self,
        params: Parameters<UnloadFileRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        dispatch_to_service(&self.service, "sysml.unload_file", serde_json::json!({
            "uri": req.uri,
        }))
    }

    /// Load a SysML file from disk via a whole-file (batch) parse. Returns the URI key assigned to the model.
    ///
    /// Response envelope carries `_readiness` (P-RA5) so the caller can
    /// decide whether to wait for indexing before issuing follow-up
    /// queries.
    #[tool(name = "sysml_load_file")]
    async fn load_file(
        &self,
        params: Parameters<FilePathRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        let uri = req.path.clone();
        dispatch_to_service_with_readiness(
            &self.service,
            "sysml.load_file",
            serde_json::json!({ "path": req.path }),
            &uri,
        )
    }

    /// Discover and load all .sysml files under a directory (recursive). Returns loaded URIs and any errors. Use this to load an entire workspace before running cross-file queries.
    ///
    /// Response envelope carries `_readiness` (P-RA5). The readiness
    /// snapshot is queried against the workspace root path; library /
    /// project state lets the caller distinguish "still indexing" from
    /// "indexed and ready to query".
    #[tool(name = "sysml_load_workspace")]
    async fn load_workspace(
        &self,
        params: Parameters<WorkspaceRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        let uri = req.path.clone();
        dispatch_to_service_with_readiness(
            &self.service,
            "sysml.load_workspace",
            serde_json::json!({ "root": req.path }),
            &uri,
        )
    }

    /// Check all constraints in a loaded model, optionally with value overrides. Returns pass/fail results for each constraint.
    #[tool(name = "sysml_constraint_check")]
    async fn constraint_check(
        &self,
        params: Parameters<ConstraintCheckRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        let overrides: Vec<(String, String)> = req
            .overrides
            .unwrap_or_default()
            .into_iter()
            .collect();
        dispatch_to_service(&self.service, "sysml.constraint.check", serde_json::json!({
            "uri": req.uri,
            "overrides": overrides,
        }))
    }

    /// Evaluate a standalone SysML expression with optional variable bindings. Returns the computed value.
    #[tool(name = "sysml_expression_eval")]
    async fn expression_eval(
        &self,
        params: Parameters<ExpressionEvalRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        let context: Vec<(String, String)> = req
            .context
            .unwrap_or_default()
            .into_iter()
            .collect();
        dispatch_to_service(&self.service, "sysml.expression.eval", serde_json::json!({
            "expr": req.expression,
            "context": context,
        }))
    }

    /// List all currently loaded model URIs.
    #[tool(name = "sysml_loaded_uris")]
    async fn loaded_uris(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(&self.service, "sysml.loaded_uris", serde_json::json!({}))
    }

    /// Run a structured, paged element-list query. Prefer count/ids first, then summary pages, then hydrate selected ids with sysml_element or sysml_get_source.
    #[tool(name = "sysml_query")]
    async fn query(
        &self,
        params: Parameters<QueryRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        let mut spec = req.spec;
        if let Some(obj) = spec.as_object_mut() {
            obj.entry("limit".to_owned()).or_insert_with(|| serde_json::json!(100));
        }
        dispatch_to_service(&self.service, "sysml.query", serde_json::json!({
            "uri": req.uri,
            "spec": spec,
        }))
    }

    /// Find elements by name pattern (substring match), optionally filtered by kind. Legacy wrapper; prefer sysml_query for new agent workflows.
    #[tool(name = "sysml_find")]
    async fn find(
        &self,
        params: Parameters<FindRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        dispatch_to_service(&self.service, "sysml.find", serde_json::json!({
            "uri": req.uri,
            "pattern": req.pattern,
            "kind": req.kind,
        }))
    }

    /// Get a single element by its ID.
    #[tool(name = "sysml_element")]
    async fn element(
        &self,
        params: Parameters<ElementRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        dispatch_to_service(&self.service, "sysml.element", serde_json::json!({
            "uri": req.uri,
            "id": req.id,
        }))
    }

    /// Get direct children of an element (ownership hierarchy).
    #[tool(name = "sysml_children")]
    async fn children(
        &self,
        params: Parameters<ElementRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        dispatch_to_service(&self.service, "sysml.children", serde_json::json!({
            "uri": req.uri,
            "id": req.id,
        }))
    }

    /// Get the source text and span covering one element's declaration.
    ///
    /// Returns the declaration slice for sneak-peek rendering — the same
    /// shape Monaco mounts in the simulation-app's read-only editor.
    #[tool(name = "sysml_get_source")]
    async fn get_source(
        &self,
        params: Parameters<ElementRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        dispatch_to_service(&self.service, "sysml.get_source", serde_json::json!({
            "uri": req.uri,
            "id": req.id,
        }))
    }

    /// Compute element and relationship count statistics for a model.
    #[tool(name = "sysml_stats")]
    async fn stats(
        &self,
        params: Parameters<UriRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(&self.service, "sysml.stats", serde_json::json!({"uri": params.0.uri}))
    }

    /// Find all requirements that have no Verify relationship targeting them.
    #[tool(name = "sysml_unverified")]
    async fn unverified(
        &self,
        params: Parameters<UriRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(&self.service, "sysml.unverified", serde_json::json!({"uri": params.0.uri}))
    }

    /// Build a hierarchical tree of root elements and their children.
    /// Use max_depth to limit output size (e.g. max_depth=2 for top two levels).
    ///
    /// `view` defaults to `"full"` for AI-agent callers — every element
    /// kind preserved (ports, FeatureTyping, expression AST, transitions).
    /// Pass `"user_facing"` to mirror what the simulation UI sees.
    #[tool(name = "sysml_model_tree")]
    async fn model_tree(
        &self,
        params: Parameters<ModelTreeRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        // Default to `"full"` for MCP callers — AI agents inspecting
        // models often need ports for physics coupling and type
        // bindings for refactoring. The REST handler defaults to
        // user-facing instead.
        let view = req.view.unwrap_or_else(|| "full".to_string());
        let mut json = serde_json::json!({"uri": req.uri, "view": view});
        if let Some(depth) = req.max_depth {
            json["max_depth"] = serde_json::json!(depth);
        }
        dispatch_to_service(&self.service, "sysml.model.tree", json)
    }

    /// Export a loaded model as canonical SysML v2 JSON.
    #[tool(name = "sysml_export_json")]
    async fn export_json(
        &self,
        params: Parameters<UriRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(&self.service, "sysml.export.json", serde_json::json!({"uri": params.0.uri}))
    }

    /// List user-authored ViewUsage / ViewDefinition elements with their
    /// Expose memberships, render members, and filter members.
    #[tool(name = "sysml_views_list")]
    async fn views_list(
        &self,
        params: Parameters<UriRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(&self.service, "sysml.views.list", serde_json::json!({"uri": params.0.uri}))
    }

    /// Render a user-authored ViewUsage / ViewDefinition as a diagram.
    #[tool(name = "sysml_views_render")]
    async fn views_render(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.views.render", json)
    }

    /// List ViewUsages / ViewDefinitions that satisfy a given viewpoint.
    #[tool(name = "sysml_views_by_viewpoint")]
    async fn views_by_viewpoint(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.views.by_viewpoint", json)
    }

    /// List ViewpointDefinitions / ViewpointUsages whose
    /// StakeholderMembership references a given stakeholder PartUsage.
    #[tool(name = "sysml_viewpoints_by_stakeholder")]
    async fn viewpoints_by_stakeholder(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.viewpoints.by_stakeholder", json)
    }

    /// Build a `view scratch : InterconnectionView { expose ...; }`
    /// snippet from a list of qualified names. For the editor's
    /// "create view def from selection" affordance.
    #[tool(name = "sysml_views_create_scratch")]
    async fn views_create_scratch(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.views.create_scratch", json)
    }

    /// Return the full machine-readable command catalog (all service operations).
    #[tool(name = "sysml_command_catalog")]
    async fn command_catalog(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        let catalog = SysmlService::command_catalog();
        ok_json(&catalog)
    }

    /// Report service feature-flag capabilities (fork-at-tick, snapshot retention, etc.).
    #[tool(name = "sysml_system_capabilities")]
    async fn system_capabilities(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(&self.service, "sysml.system.capabilities", serde_json::json!({}))
    }

    /// Workspace-level model-content capability flags (state machines,
    /// actions, constraints, requirements, trade studies, port flows,
    /// ODE dynamics) plus the named-element lists the simulation app
    /// uses to populate selectors. Workspace-scoped — operates on the
    /// loaded user-project file set.
    #[tool(name = "sysml_workspace_capabilities")]
    async fn workspace_capabilities(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(
            &self.service,
            "sysml.workspace.capabilities",
            serde_json::json!({}),
        )
    }

    // -- Navigation queries --

    /// Get all ancestors (parent, grandparent, ...) of an element up to the root.
    #[tool(name = "sysml_ancestors")]
    async fn ancestors(
        &self,
        params: Parameters<ElementRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        dispatch_to_service(&self.service, "sysml.ancestors", serde_json::json!({
            "uri": req.uri,
            "id": req.id,
        }))
    }

    /// Get all descendants (children, grandchildren, ...) of an element recursively.
    #[tool(name = "sysml_descendants")]
    async fn descendants(
        &self,
        params: Parameters<ElementRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        dispatch_to_service(&self.service, "sysml.descendants", serde_json::json!({
            "uri": req.uri,
            "id": req.id,
        }))
    }

    /// Generate a traceability matrix showing relationships between element kinds. Returns rows with source, target, and linking relationship.
    #[tool(name = "sysml_trace_matrix")]
    async fn trace_matrix(
        &self,
        params: Parameters<TraceMatrixRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        dispatch_to_service(&self.service, "sysml.trace_matrix", serde_json::json!({
            "uri": req.uri,
            "source_kind": req.source_kind,
            "rel_kind": req.relation_kind,
            "target_kind": req.target_kind,
        }))
    }

    /// Requirements-workbench table rows (B2): document-ordered requirement
    /// rows over the elaborated workspace with text, links, and rollups.
    #[tool(name = "sysml_workspace_requirement_rows", description = "Requirements-workbench table rows: document-ordered Requirement{Definition,Usage} rows over the elaborated workspace, with statement text (doc bodies), requirement ID (short name), outline depth, StatusInfo maturity, satisfied_by/verified_by/derived_from/derives/refines links, and a three-state verification rollup (fail/incomplete/pass). Paged: spec is {filter?, limit?, cursor?}; omit limit for MCP's context-safe default of 100 rows.")]
    async fn workspace_requirement_rows(
        &self,
        params: Parameters<RequirementRowsRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        let mut spec = req.spec.unwrap_or_else(|| serde_json::json!({}));
        if let Some(obj) = spec.as_object_mut() {
            obj.entry("limit".to_owned()).or_insert_with(|| serde_json::json!(100));
        }
        dispatch_to_service(
            &self.service,
            "sysml.workspace.requirement_rows",
            serde_json::json!({ "spec": spec }),
        )
    }

    /// Per-element requirement contract detail (R18 / B2.1).
    #[tool(name = "sysml_workspace_requirement_detail", description = "Evaluated contract of one requirement (pass a row id from sysml_workspace_requirement_rows): subject, assumed/required constraints (inline text is verbatim source; reference-form constraints link their definition when the name resolves unambiguously), owned attribute values, plus narrative buckets (actors, stakeholders, framed concerns, rationale). Verdict inputs (subject/constraints/values) are separated from narrative roles by design. Errors on unknown ids and non-requirement elements.")]
    async fn workspace_requirement_detail(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.requirement_detail", json)
    }

    /// Buffer-writeback field edits (workbench design §7.2): the service
    /// COMPUTES a guarded TextEdit; the caller applies it to the editor
    /// buffer only when the buffer slice equals `expected_old_text`.
    #[tool(name = "sysml_workspace_edit_requirement_doc", description = "Compute a guarded text edit replacing an element's doc-comment body (params: element_id, new_text). Returns FieldEditComputed { uri, element_id, field, edit: { line/col UTF-16 range, new_text, expected_old_text } }. The caller applies it to the source buffer ONLY if the buffer slice equals expected_old_text (stale-buffer guard). Fails when the element has no doc comment — adding one is a creation action, not an edit.")]
    async fn workspace_edit_requirement_doc(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.edit_requirement_doc", json)
    }

    #[tool(name = "sysml_workspace_edit_attribute_value", description = "Compute a guarded text edit replacing an attribute usage's inline `= value` expression (params: element_id, new_value — single line, no `;`). Returns FieldEditComputed with an expected_old_text stale-buffer guard. Fails when the declaration has no inline value — adding one is a creation action, not an edit.")]
    async fn workspace_edit_attribute_value(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.edit_attribute_value", json)
    }

    #[tool(name = "sysml_workspace_edit_requirement_maturity", description = "Compute a guarded text edit setting an element's @StatusInfo status to StatusKind::<status> (params: element_id, status ∈ open|tbd|tbr|tbc|done|closed — the spec's closed vocabulary; invalid values are rejected at the boundary). Returns FieldEditComputed with an expected_old_text stale-buffer guard. Fails when the element has no @StatusInfo metadata — adding one is a creation action, not an edit. Maturity is MODEL state; approval/review lifecycle lives in the workflow sidecar, never in source.")]
    async fn workspace_edit_requirement_maturity(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(
            &self.service,
            "sysml.workspace.edit_requirement_maturity",
            json,
        )
    }

    #[tool(name = "sysml_workspace_create_requirement", description = "Compute a guarded text edit inserting a NEW requirement (params: parent_id — a package or requirement, name, optional short_name reqId rendered <'…'>, optional doc statement text) at the end of the parent's body. Server-side printed shape (sysml_core::member_print) — never a client template. Returns FieldEditComputed with an expected_old_text stale-buffer guard; the caller splices into the source buffer (editor owns save).")]
    async fn workspace_create_requirement(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.create_requirement", json)
    }

    #[tool(name = "sysml_workspace_add_requirement_doc", description = "Compute a guarded text edit ADDING a doc comment to an element that has none (params: element_id, new_text). Fails when a doc already exists — use sysml_workspace_edit_requirement_doc for edits. Returns FieldEditComputed with an expected_old_text stale-buffer guard.")]
    async fn workspace_add_requirement_doc(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.add_requirement_doc", json)
    }

    #[tool(name = "sysml_workspace_add_requirement_maturity", description = "Compute a guarded text edit ADDING @StatusInfo { status = StatusKind::<status> } maturity metadata to an element that has none (params: element_id, status ∈ open|tbd|tbr|tbc|done|closed). Fails when @StatusInfo already exists — use sysml_workspace_edit_requirement_maturity for edits. Returns FieldEditComputed with an expected_old_text stale-buffer guard.")]
    async fn workspace_add_requirement_maturity(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(
            &self.service,
            "sysml.workspace.add_requirement_maturity",
            json,
        )
    }

    #[tool(name = "sysml_workspace_add_requirement_role", description = "Compute a guarded text edit adding a `<keyword> <name> : <Type>;` typed member (params: requirement_id, role = \"subject\"|\"actor\"|\"stakeholder\"|\"concern\", type_id, name). subject accepts any definition, actor/stakeholder a part definition, concern a concern definition; subject is singleton. Returns FieldEditComputed with an expected_old_text stale-buffer guard.")]
    async fn workspace_add_requirement_role(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.add_requirement_role", json)
    }

    #[tool(name = "sysml_workspace_add_constraint", description = "Compute a guarded text edit adding an `assume/require constraint [name] { <expr> }` member to a requirement (params: element_id, kind = \"assume\"|\"require\", expr, name?). expr is a single-line boolean expression (no braces or `;`). Returns FieldEditComputed with an expected_old_text stale-buffer guard.")]
    async fn workspace_add_constraint(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.add_constraint", json)
    }

    #[tool(name = "sysml_workspace_add_attribute", description = "Compute a guarded text edit adding an `attribute <name> [= <value>];` member to an element (params: element_id, name, value?). Name is an identifier; value (optional) is a single-line expression (no `;`). Fails when an attribute of that name already exists (edit its value with sysml_workspace_edit_attribute_value). Returns FieldEditComputed with an expected_old_text stale-buffer guard.")]
    async fn workspace_add_attribute(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.add_attribute", json)
    }

    #[tool(name = "sysml_workspace_add_rationale", description = "Compute a guarded text edit adding a @Rationale { text = \"…\" } metadata member to an element (params: element_id, text). Add-only (a requirement may carry several rationale annotations). Text is single-line; embedded quotes/backslashes are escaped. Returns FieldEditComputed with an expected_old_text stale-buffer guard.")]
    async fn workspace_add_rationale(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.add_rationale", json)
    }

    #[tool(name = "sysml_workspace_add_satisfy_link", description = "Compute a guarded text edit inserting `satisfy <requirement>;` at the end of the picked subject element's body (params: requirement_id, subject_id). The edit targets the SUBJECT's file — possibly a different file than the requirement's. Fails hard when the satisfy link already exists. Returns FieldEditComputed with an expected_old_text stale-buffer guard.")]
    async fn workspace_add_satisfy_link(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.add_satisfy_link", json)
    }

    #[tool(name = "sysml_workspace_add_verify_link", description = "Compute a guarded text edit inserting `verify <requirement>;` into the picked verification case's objective body (params: requirement_id, case_id). A case with no objective gets the whole `objective { verify …; }` block in one insertion. The edit targets the CASE's file. Fails hard on duplicate links or non-case targets. Returns FieldEditComputed with an expected_old_text stale-buffer guard.")]
    async fn workspace_add_verify_link(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.add_verify_link", json)
    }

    #[tool(name = "sysml_workspace_add_derive_link", description = "Compute a guarded text edit inserting a `#derivation connection { end #original ::> <original>; end #derive ::> <derived>; }` block at the end of the DERIVED requirement's owning package (params: requirement_id = the derived end, original_id). Prepends `private import RequirementDerivation::*;` in the same insertion when the owning-package chain lacks one. Fails hard when the derive link already exists. Returns FieldEditComputed with an expected_old_text stale-buffer guard.")]
    async fn workspace_add_derive_link(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.add_derive_link", json)
    }

    #[tool(name = "sysml_workspace_add_refine_link", description = "Compute a guarded text edit inserting a `dependency from <refining> to <refined> { @Refinement; }` at the end of the REFINING requirement's owning package (params: requirement_id = the refining end, refined_id). Refine is a KerML Dependency + ModelingMetadata::Refinement, not a keyword. Prepends `private import ModelingMetadata::*;` in the same insertion when the owning-package chain lacks one. Fails hard when the refine link already exists. Returns FieldEditComputed with an expected_old_text stale-buffer guard.")]
    async fn workspace_add_refine_link(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.add_refine_link", json)
    }

    /// Multi-URI peer of sysml_model_tree. Walks every loaded user file and
    /// returns one {uri, nodes} group per URI, deterministically ordered.
    /// Each node carries an LSP-style {start, end} range (line + UTF-16
    /// character) resolved at the service boundary.
    #[tool(name = "sysml_workspace_model_tree", description = "Multi-URI peer of sysml_model_tree. Walks all loaded user files; returns Vec<{uri, nodes: [...]}> with deterministic per-URI ordering. Each node has LSP-style {start, end} line/character ranges. Pass max_depth to limit recursion (omit for unlimited). Pass view=\"full\" (default) to keep every element kind or \"user_facing\" to mirror the simulation UI projection.")]
    async fn workspace_model_tree(
        &self,
        params: Parameters<WorkspaceModelTreeRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        let view = req.view.unwrap_or_else(|| "full".to_string());
        let mut json = serde_json::json!({ "view": view });
        if let Some(depth) = req.max_depth {
            json["max_depth"] = serde_json::json!(depth);
        }
        dispatch_to_service(&self.service, "sysml.workspace.model_tree", json)
    }

    // -- Simulation session --

    /// Start a state machine simulation session. Returns a session_key (use in subsequent step/stop calls) and the initial step result with the current state.
    #[tool(name = "sysml_simulate_start")]
    async fn simulate_start(
        &self,
        params: Parameters<SimulateStartRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        dispatch_to_service(&self.service, "sysml.simulate.start", serde_json::json!({
            "uri": req.uri,
            "sm_name": req.sm_name,
        }))
    }

    /// Step a running state machine simulation forward, optionally injecting an event trigger. Returns the new state and any outputs.
    #[tool(name = "sysml_simulate_step")]
    async fn simulate_step(
        &self,
        params: Parameters<SimulateStepRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        dispatch_to_service(&self.service, "sysml.simulate.step", serde_json::json!({
            "session_key": req.session_key,
            "event": req.event,
        }))
    }

    /// Stop and discard a running state machine simulation session, freeing resources.
    #[tool(name = "sysml_simulate_stop")]
    async fn simulate_stop(
        &self,
        params: Parameters<SessionKeyRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        dispatch_to_service(&self.service, "sysml.simulate.stop", serde_json::json!({
            "session_key": req.session_key,
        }))
    }

    // -- Action session --

    /// Start an action execution session. Returns a session_key for use in subsequent sysml_action_step calls.
    #[tool(name = "sysml_action_start")]
    async fn action_start(
        &self,
        params: Parameters<ActionStartRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        dispatch_to_service(&self.service, "sysml.action.start", serde_json::json!({
            "uri": req.uri,
            "action_name": req.action_name,
        }))
    }

    /// Step an action execution session forward. Returns the trace entry with current node, outputs, and completion status.
    #[tool(name = "sysml_action_step")]
    async fn action_step(
        &self,
        params: Parameters<SessionKeyRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = params.0;
        dispatch_to_service(&self.service, "sysml.action.step", serde_json::json!({
            "session_key": req.session_key,
        }))
    }

    // -- Session catalog (kind-generic) --

    /// List all live runtime sessions as typed summaries. Polls the full catalog.
    #[tool(name = "sysml_sessions_list")]
    async fn sessions_list(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(&self.service, "sysml.sessions.list", serde_json::json!({}))
    }

    /// Create an execution session, inferring its kind (simulation / action /
    /// orchestrator) from the model and the optional `target`. The unified
    /// creation entry point subsuming the kind-specific `*.start` commands.
    #[tool(name = "sysml_sessions_create")]
    async fn sessions_create(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.sessions.create", json)
    }

    /// Fetch full detail for a single session including subsystems and latest snapshot.
    #[tool(name = "sysml_sessions_info")]
    async fn sessions_info(
        &self,
        params: Parameters<SessionIdRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(&self.service, "sysml.sessions.info", serde_json::json!({
            "session_id": params.0.session_id,
        }))
    }

    /// Drop all sessions past the inactivity timeout. Returns the count removed.
    #[tool(name = "sysml_sessions_reap")]
    async fn sessions_reap(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(&self.service, "sysml.sessions.reap", serde_json::json!({}))
    }

    /// Report per-kind session budgets and current usage.
    #[tool(name = "sysml_sessions_quota")]
    async fn sessions_quota(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(&self.service, "sysml.sessions.quota", serde_json::json!({}))
    }

    /// Stop and remove a session of any kind.
    #[tool(name = "sysml_sessions_stop")]
    async fn sessions_stop(
        &self,
        params: Parameters<SessionIdRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(&self.service, "sysml.sessions.stop", serde_json::json!({
            "session_id": params.0.session_id,
        }))
    }

    /// Advance any session, optionally injecting an event into the primary
    /// subsystem and applying context overrides ONCE before stepping. Pass
    /// `ticks` (default 1) to advance that many ticks server-side in one call
    /// (for fine-dt runs needing many ticks to reach an event); the run still
    /// stops early at a breakpoint or the session's configured tick/time limit.
    #[tool(name = "sysml_sessions_step")]
    async fn sessions_step(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.sessions.step", json)
    }

    /// Inject an event into a named subsystem of any session and advance one
    /// tick, optionally applying context overrides before stepping.
    #[tool(name = "sysml_sessions_inject")]
    async fn sessions_inject(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.sessions.inject", json)
    }

    /// Verify a running session's declared verification cases against its LIVE
    /// state (reads simulation-produced attributes like tripped/trip_time from
    /// the session's orchestrator context + slot store), routing each case
    /// through the one VerificationRunner. Optional `case_names` filters.
    #[tool(name = "sysml_sessions_verify")]
    async fn sessions_verify(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.sessions.verify", json)
    }

    /// Reset any session to its initial state and restart the expiry timer.
    #[tool(name = "sysml_sessions_reset")]
    async fn sessions_reset(
        &self,
        params: Parameters<SessionIdRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(&self.service, "sysml.sessions.reset", serde_json::json!({
            "session_id": params.0.session_id,
        }))
    }

    /// Resume a session paused at a breakpoint (clears the pause; keeps
    /// breakpoints armed). Idempotent on an already-running session.
    #[tool(name = "sysml_sessions_resume")]
    async fn sessions_resume(
        &self,
        params: Parameters<SessionIdRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(&self.service, "sysml.sessions.resume", serde_json::json!({
            "session_id": params.0.session_id,
        }))
    }

    /// Set the display label on a session.
    #[tool(name = "sysml_sessions_rename")]
    async fn sessions_rename(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.sessions.rename", json)
    }

    /// List the subsystems of a session without fetching a full snapshot.
    #[tool(name = "sysml_sessions_subsystems")]
    async fn sessions_subsystems(
        &self,
        params: Parameters<SessionIdRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(&self.service, "sysml.sessions.subsystems", serde_json::json!({
            "session_id": params.0.session_id,
        }))
    }

    /// List the variable names captured in a session's canonical
    /// time-series buffer.
    #[tool(name = "sysml_sessions_timeseries_names")]
    async fn sessions_timeseries_names(
        &self,
        params: Parameters<SessionIdRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(&self.service, "sysml.sessions.timeseries_names", serde_json::json!({
            "session_id": params.0.session_id,
        }))
    }

    /// Fetch (time_ms, value) points for one variable from a session's
    /// canonical time-series buffer (optionally bounded by start_ms /
    /// end_ms).
    #[tool(name = "sysml_sessions_timeseries")]
    async fn sessions_timeseries(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.sessions.timeseries", json)
    }

    /// Fetch an LTTB-decimated time series for one variable (preserves
    /// visual shape; ideal for chart rendering at ~target_points
    /// on-screen pixels).
    #[tool(name = "sysml_sessions_timeseries_decimated")]
    async fn sessions_timeseries_decimated(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.sessions.timeseries_decimated", json)
    }

    /// Fork a session at its current tick into an independent child.
    #[tool(name = "sysml_sessions_fork")]
    async fn sessions_fork(
        &self,
        params: Parameters<SessionIdRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(&self.service, "sysml.sessions.fork", serde_json::json!({
            "session_id": params.0.session_id,
        }))
    }

    /// Fork a session and atomically apply parameter overrides to the child.
    #[tool(name = "sysml_sessions_fork_with_overrides")]
    async fn sessions_fork_with_overrides(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.sessions.fork_with_overrides", json)
    }

    /// Diff two sessions' latest snapshots.
    #[tool(name = "sysml_sessions_diff")]
    async fn sessions_diff(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.sessions.diff", json)
    }

    /// Walk two sessions' histories tick-by-tick and report where they
    /// first diverged.
    #[tool(name = "sysml_sessions_diff_timeline")]
    async fn sessions_diff_timeline(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.sessions.diff_timeline", json)
    }

    /// Return the structural topology of a session — modules, subsystems,
    /// physics domains, and health status for the multi-physics simulation UI.
    #[tool(name = "sysml_sessions_topology")]
    async fn sessions_topology(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.sessions.topology", json)
    }

    // -- Session archive (R4.1) --

    /// List archived (completed) sessions matching optional workspace /
    /// origin / since / only_golden filters.
    #[tool(name = "sysml_sessions_archive_list")]
    async fn sessions_archive_list(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.sessions.archive.list", json)
    }

    /// Fetch the full archived session record (metadata + snapshots + verdicts) by id.
    #[tool(name = "sysml_sessions_archive_get")]
    async fn sessions_archive_get(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.sessions.archive.get", json)
    }

    /// Tag an archived session as a golden reference run — pins it against eviction.
    #[tool(name = "sysml_sessions_archive_mark_golden")]
    async fn sessions_archive_mark_golden(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.sessions.archive.mark_golden", json)
    }

    /// Remove the golden tag from an archived session — it becomes eligible
    /// for LRU eviction again.
    #[tool(name = "sysml_sessions_archive_unmark_golden")]
    async fn sessions_archive_unmark_golden(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.sessions.archive.unmark_golden", json)
    }

    // -- Analysis --

    /// Per-URI readiness predicate: is this question answerable yet
    /// (library/project/file)? Cheap derivation — no new locks, no
    /// indexing kicked off. Use this from agent flows to decide
    /// whether to retry a "still loading" tool response.
    #[tool(name = "sysml_readiness")]
    async fn readiness(
        &self,
        params: Parameters<UriRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(&self.service, "sysml.readiness", serde_json::json!({
            "uri": params.0.uri,
        }))
    }

    /// Get diagnostics (errors, warnings) for a loaded model URI.
    ///
    /// Response envelope carries `_readiness` (P-RA5) so AI agents can
    /// distinguish "no diagnostics yet because still indexing" from
    /// "no diagnostics found".
    #[tool(name = "sysml_diagnostics")]
    async fn diagnostics(
        &self,
        params: Parameters<UriRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let uri = params.0.uri;
        dispatch_to_service_with_readiness(
            &self.service,
            "sysml.diagnostics",
            serde_json::json!({ "uri": uri }),
            &uri,
        )
    }

    /// Compute the inspect-pipeline result (diagnostics + semantic tokens)
    /// for one URI or every loaded user URI.
    #[tool(name = "sysml_inspect")]
    async fn inspect(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.inspect", json)
    }

    /// Build the document outline (nested symbol tree) for a loaded URI.
    #[tool(name = "sysml_outline")]
    async fn outline(
        &self,
        params: Parameters<UriRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        dispatch_to_service(&self.service, "sysml.outline", serde_json::json!({"uri": params.0.uri}))
    }

    /// Find every reference to the element at the given cursor position.
    #[tool(name = "sysml_references")]
    async fn references(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.references", json)
    }

    /// Resolve the goto-definition target for the cursor position.
    #[tool(name = "sysml_goto_definition")]
    async fn goto_definition(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.goto_definition", json)
    }

    /// Render hover content (markdown + range) for the cursor position.
    ///
    /// Response envelope carries `_readiness` (P-RA5) — particularly
    /// useful for hover/completion which return empty results while the
    /// workspace is still indexing.
    #[tool(name = "sysml_hover")]
    async fn hover(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        let uri = json
            .get("uri")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        dispatch_to_service_with_readiness(&self.service, "sysml.hover", json, &uri)
    }

    /// Compute completion candidates for the cursor position.
    ///
    /// Response envelope carries `_readiness` (P-RA5).
    #[tool(name = "sysml_completion")]
    async fn completion(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        let uri = json
            .get("uri")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        dispatch_to_service_with_readiness(&self.service, "sysml.completion", json, &uri)
    }

    /// Enrich a completion item with documentation and type detail.
    #[tool(name = "sysml_completion_resolve")]
    async fn completion_resolve(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.completion.resolve", json)
    }

    /// Compute prepare-rename info or apply-rename workspace edits at a cursor position.
    #[tool(name = "sysml_rename")]
    async fn rename(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.rename", json)
    }

    /// Compute whitespace-only formatting edits for a loaded document.
    #[tool(name = "sysml_format_document")]
    async fn format_document(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.format.document", json)
    }

    /// Compute every code action (quick-fixes, refactorings, source actions) at a range.
    #[tool(name = "sysml_code_action_list")]
    async fn code_action_list(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.code_action.list", json)
    }

    /// Compute the workspace edit for a diagram-driven create/delete/editLabel/addSequence action.
    #[tool(name = "sysml_diagram_edit")]
    async fn diagram_edit(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The whole request is the body — wrap it in `{"request": <body>}`
        // so the typed request struct (which has a single `request: Value` field)
        // deserializes cleanly. MCP clients pass the raw DiagramEditRequest at
        // the top level for ergonomics.
        let raw = serde_json::to_value(params.0.params).unwrap_or_default();
        let wrapped = serde_json::json!({ "request": raw });
        dispatch_to_service(&self.service, "sysml.diagram.edit", wrapped)
    }

    /// Open an ad-hoc diagram for a URI and return its ViewModel. Updates open-diagram
    /// state so subsequent edits refresh the same view.
    #[tool(name = "sysml_diagram_open", description = "Open a diagram for the given URI + view_type. Returns the renderer-neutral ViewModel. Updates open-diagram state for auto-refresh on file change. view_type defaults to \"general\"; accepts general | interconnection | state | action | browser | sequence | grid | geometry.")]
    async fn diagram_open(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.diagram.open", json)
    }

    /// Switch the diagram view type for a URI and return its ViewModel.
    #[tool(name = "sysml_diagram_view", description = "Switch a diagram's view type for the given URI. Returns the renderer-neutral ViewModel. Accepts view_type: general | interconnection | state | action | browser | sequence | grid | geometry.")]
    async fn diagram_view(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.diagram.view", json)
    }

    /// Return the renderer-agnostic ViewModel (scene + tokens + text-map + interactions).
    #[tool(name = "sysml_diagram_viewmodel", description = "Return the renderer-agnostic ViewModel for a DECLARED view as JSON: the diagram scene (geometry/structure) plus design tokens (color palette), the ElementId<->Span text-map, interaction descriptors (e.g. go-to-definition targets), and the framed-view metadata. The canonical wire artifact for the new renderer; mirrors sysml_views_render but returns the ViewModel. Params: uri (model uri), view_usage_id (ElementId of the ViewUsage/ViewDefinition to render — get one from sysml_views_list), expanded_ids (node ids to show expanded). The scene is scoped by the view's Expose/filter memberships.")]
    async fn diagram_view_model(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.diagram.viewmodel", json)
    }

    /// Return the per-tick simulation overlay for a live session.
    #[tool(name = "sysml_diagram_sim_overlay", description = "Return the per-tick simulation overlay for a live session as JSON, joined to a DECLARED view's diagram scene by ElementId: active-element highlights (active SM substates / completed subsystems), live scalar value badges, and the time-series channel directory (with element links and latest readings) for plotting. Session-scoped companion to sysml_diagram_viewmodel. Params: session_id, view_usage_id (MUST match the sysml_diagram_viewmodel call so the overlay joins the same scene), expanded_ids. Fails if the session is unknown or has not produced a snapshot yet.")]
    async fn diagram_sim_overlay(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.diagram.sim_overlay", json)
    }

    /// Return the per-run verdict overlay for a live session.
    #[tool(name = "sysml_diagram_verdict_overlay", description = "Return the per-run verdict overlay for a live session as JSON, joined to a DECLARED view's diagram scene by ElementId: constraint solver pass/fail verdicts and the solved scalar value behind each. Session-scoped companion verdict sidecar to sysml_diagram_viewmodel. Params: session_id, view_usage_id (MUST match the sysml_diagram_viewmodel call so the overlay joins the same scene), expanded_ids. Fails if the session is unknown or has not produced a snapshot yet.")]
    async fn diagram_verdict_overlay(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.diagram.verdict_overlay", json)
    }

    /// Return the diagnostics overlay for a DECLARED view's scene, joined by ElementId.
    #[tool(name = "sysml_diagram_diagnostic_overlay", description = "Return the diagnostics overlay for a DECLARED view's diagram scene as JSON, joined by ElementId: which scene elements carry validation diagnostics, with worst-case severity (error|warning|info) per element for the badge plus every message/code for the tooltip. Diagnostics companion to sysml_diagram_viewmodel — pass the SAME view_usage_id so the overlay joins the same scene. Reads workspace diagnostics (readiness-gated); needs no session. Params: view_usage_id, expanded_ids (MUST match the sysml_diagram_viewmodel call so scene ids align).")]
    async fn diagram_diagnostic_overlay(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.diagram.diagnostic_overlay", json)
    }

    /// Re-project a diagram with a caller-supplied expanded-node set.
    #[tool(name = "sysml_diagram_expand", description = "Re-project a diagram with a caller-supplied full expanded-node set; replaces the URI's expanded_nodes state. Returns the renderer-neutral ViewModel.")]
    async fn diagram_expand(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.diagram.expand", json)
    }

    /// Render flow connections for a URI as PlantUML sequence text.
    #[tool(name = "sysml_flow_visualize", description = "Render the flow connections for the given URI as PlantUML sequence text. flow_id is reserved for narrowing to a single flow (today renders all).")]
    async fn flow_visualize(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.flow.visualize", json)
    }

    /// Render a named action's control flow as PlantUML activity text.
    #[tool(name = "sysml_action_visualize", description = "Render a named action's control flow as PlantUML activity text.")]
    async fn action_visualize(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.action.visualize", json)
    }

    /// Run a verification case end-to-end (state-machine compile + event-script
    /// extraction + auto-step + per-tick assertion eval). Returns the verdict,
    /// requirement results, per-tick assertion checkpoints, full execution
    /// trace, and final snapshot.
    #[tool(name = "sysml_scenario_run", description = "Run a verification case end-to-end (SM compile + event-script + auto-step + per-tick assertion eval). Returns verdict + trace + assertion_checkpoints.")]
    async fn scenario_run(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.scenario.run", json)
    }

    /// Return the full execution trace (every TickSnapshot in `history()`) for
    /// a runtime session as JSON.
    #[tool(name = "sysml_timeline_getTrace", description = "Return the full execution trace for a runtime session — every TickSnapshot from history() serialized to JSON.")]
    async fn timeline_get_trace(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.timeline.getTrace", json)
    }

    /// Return a single TickSnapshot from a session's history at the given
    /// tick index.
    #[tool(name = "sysml_timeline_getSnapshot", description = "Return a single TickSnapshot from a runtime session's history at the given tick index.")]
    async fn timeline_get_snapshot(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.timeline.getSnapshot", json)
    }

    // -- Orchestrator session --

    /// Start a multi-subsystem orchestrator session from the model.
    #[tool(name = "sysml_orchestrate_start", description = "Start a multi-subsystem orchestrator session from the model")]
    async fn orchestrate_start(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.orchestrate.start", json)
    }

    /// Compile all subsystems from the workspace and start an orchestrator session.
    #[tool(name = "sysml_orchestrate_workspace_start", description = "Compile all state machines and ODEs from the workspace and start an orchestrator session")]
    async fn orchestrate_workspace_start(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.orchestrate.workspace.start", json)
    }

    /// Advance all subsystems in the orchestrator by one tick.
    #[tool(name = "sysml_orchestrate_step", description = "Advance all subsystems in the orchestrator by one tick")]
    async fn orchestrate_step(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.orchestrate.step", json)
    }

    /// Inject an event into a specific subsystem and advance the orchestrator.
    #[tool(name = "sysml_orchestrate_inject", description = "Inject an event into a specific subsystem and advance the orchestrator")]
    async fn orchestrate_inject(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.orchestrate.inject", json)
    }

    // `sysml_simulate_continuous_start` was removed alongside the
    // `sysml.simulate.continuous.start` service command (model-bypassing
    // explicit-ODE-string path; execution-entry-unification-plan.md P5). Use
    // `sysml_simulate_continuous_auto` or `sysml_sessions_create`.

    /// Start a continuous simulation auto-discovering ODE config from model metadata.
    #[tool(name = "sysml_simulate_continuous_auto", description = "Start a continuous simulation auto-discovering ODE config from model metadata")]
    async fn simulate_continuous_auto(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.simulate.continuous.auto", json)
    }
    /// Stop and remove an orchestrate session.
    #[tool(name = "sysml_orchestrate_stop", description = "Terminate an orchestrate session by session key")]
    async fn orchestrate_stop(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.orchestrate.stop", json)
    }

    // -- Verification & Analysis --

    /// Run a named verification case against model requirements.
    #[tool(name = "sysml_verify", description = "Run a named verification case against model requirements with optional parameter overrides")]
    async fn verify(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.verify", json)
    }

    /// Return the verdict timeline across past verification runs.
    #[tool(name = "sysml_verify_timeline", description = "Return verdict-flip history across past verification runs of the current workspace (scoped server-side via session provenance), with optional case and timestamp filters")]
    async fn verify_timeline(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.verify.timeline", json)
    }

    /// Ingest externally produced verification verdicts (B10).
    #[tool(name = "sysml_verify_record_external", description = "Ingest externally produced verification verdicts (CI, pytest, HIL) as a synthetic archived session (origin 'external'). Requires `tool`, `declared_digest` (the model content digest the results were produced against), and `verdicts`: [{case_id: <verification case NAME>, verdict: pass|fail|inconclusive|error, artifacts?}]. Unknown case names or verdict strings reject the whole batch; a stale declared_digest is recorded and labeled, never rejected.")]
    async fn verify_record_external(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.verify.record_external", json)
    }

    /// List verification executions (projection over the session archive).
    #[tool(name = "sysml_verify_executions", description = "List verification executions (newest-first) as a projection over the session archive: each is an archived session (trajectory or external ingest) carrying >=1 verdict, with origin, evaluation_mode, B6 provenance, external run identity, per-case results (verdict + case_changed_since stale flag), and verdict counts. Verdict-less simulation runs are not executions. Optional `case_name` keeps only executions touching that case. Scoped server-side via session provenance, like sysml_verify_timeline.")]
    async fn verify_executions(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.verify.executions", json)
    }

    /// Per-case latest verification status across executions (per-mode).
    #[tool(name = "sysml_verify_latest_status", description = "Per-case latest verification status across executions, context-qualified by evaluation_mode: {trajectory?, external?} with verdict, execution_id, timestamp, case_changed_since, and mode-specific provenance (trajectory model_digest; external tool + matches_current_model). Execution-side only — compose with the static read. Scoped server-side via session provenance.")]
    async fn verify_latest_status(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.verify.latest_status", json)
    }

    /// Run a named analysis case using the solver registry.
    #[tool(name = "sysml_analysis_run", description = "Run a named analysis case using the solver registry with optional parameter overrides")]
    async fn analysis(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.analysis.run", json)
    }

    /// Run a trade study evaluating design alternatives against an objective.
    #[tool(name = "sysml_trade_study", description = "Run a trade study evaluating design alternatives against a minimize/maximize objective")]
    async fn trade_study(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.trade_study", json)
    }

    /// Run Monte Carlo constraint analysis with parameter distributions, statistics, and histograms.
    #[tool(name = "sysml_montecarlo_run", description = "Run Monte Carlo constraint analysis with parameter distributions, statistics, and histograms")]
    async fn montecarlo(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.montecarlo.run", json)
    }

    /// Run constraint network solving with binding propagation and DOF analysis.
    #[tool(name = "sysml_solve", description = "Run constraint network solving with binding propagation, DOF analysis, optional rollup and sensitivity sweep")]
    async fn solve(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.solve", json)
    }

    /// Generate a sequence trace by simulating message flow.
    #[tool(name = "sysml_trace", description = "Generate a sequence trace by simulating message flow through compiled flow topology")]
    async fn trace_sequence(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.trace", json)
    }

    /// Inspect port connections, flow paths, and test delivery.
    #[tool(name = "sysml_flow_inspect", description = "Inspect port connections, flow paths, and optionally inject a payload to test delivery")]
    async fn flow_inspect(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.flow.inspect", json)
    }

    // -- Export --

    /// Export a loaded model as PlantUML notation.
    #[tool(name = "sysml_export_plantuml", description = "Export a loaded model as PlantUML notation for general view")]
    async fn export_plantuml(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.export.plantuml", json)
    }

    // -- Store --

    /// Store a model snapshot with version metadata.
    #[tool(name = "sysml_store_save", description = "Store a model snapshot with version metadata")]
    async fn store_save(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.store.save", json)
    }

    /// Load a stored model snapshot by project and commit ID.
    #[tool(name = "sysml_store_load", description = "Load a stored model snapshot by project and commit ID")]
    async fn store_load(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.store.load", json)
    }

    /// Get the latest commit ID for a project from the store.
    #[tool(name = "sysml_store_latest", description = "Get the latest commit ID for a project from the store")]
    async fn store_latest(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.store.latest", json)
    }

    /// List all projects in the store.
    #[tool(name = "sysml_store_projects", description = "List all projects in the store")]
    async fn store_projects(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.store.projects", json)
    }

    /// List all commits for a project (most recent first).
    #[tool(name = "sysml_store_history", description = "List all commits for a project (most recent first)")]
    async fn store_history(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.store.history", json)
    }

    /// Element-level diff between two stored snapshots (baseline name or commit id refs).
    #[tool(name = "sysml_store_diff", description = "Element-level diff between two stored snapshots. `from`/`to` accept a baseline name (resolved first) or commit id; omitted `to` = latest commit. Optional element_ids narrows the result.")]
    async fn store_diff(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.store.diff", json)
    }

    /// Create a named, immutable baseline pointing at a commit.
    #[tool(name = "sysml_store_baseline_create", description = "Create a named, immutable baseline pointing at a commit (default: latest). Baselines can never be renamed or retargeted.")]
    async fn store_baseline_create(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.store.baseline.create", json)
    }

    /// List a project's baselines (most recently created first).
    #[tool(name = "sysml_store_baseline_list", description = "List a project's baselines (most recently created first)")]
    async fn store_baseline_list(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.store.baseline.list", json)
    }

    /// Snapshot the current elaborated workspace into the store.
    #[tool(name = "sysml_store_save_workspace", description = "Snapshot the current elaborated workspace into the store under a content-addressed commit id. Idempotent: an unchanged workspace returns the existing commit's metadata.")]
    async fn store_save_workspace(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.store.save_workspace", json)
    }

    /// Suspect requirement attribution between two stored snapshots (R9).
    #[tool(name = "sysml_workspace_requirement_suspects", description = "Requirement rows suspect against a baseline: diff two stored snapshots, attribute changes to their nearest owning requirement, propagate downstream along Derive edges. `from`/`to` accept a baseline name or commit id; omitted `to` = latest commit. Records carry `cleared_by` when a non-superseded clearing attestation covers them.")]
    async fn workspace_requirement_suspects(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.requirement_suspects", json)
    }

    /// Record a suspect-clearing attestation for a requirement.
    #[tool(name = "sysml_workflow_attest_suspect_clearing", description = "Attest that a suspect requirement's intent is unchanged vs a baseline. Pins the current content commit as attested_commit; later changes supersede the attestation. Requires an explicit `actor` — no default identity.")]
    async fn workflow_attest_suspect_clearing(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workflow.attest_suspect_clearing", json)
    }

    /// Record a manual-verification attestation on an element.
    #[tool(name = "sysml_workflow_attest_verification", description = "Record a MANUAL verification act on an element (attestation, never a computed verdict): `actor` verified `element_id` by `method` (inspect | analyze | demo | test — the spec's VerificationMethodKind, validated). Pins the current content commit as attested_commit; later content changes supersede the attestation. Requires non-blank `actor` and `statement`.")]
    async fn workflow_attest_verification(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workflow.attest_verification", json)
    }

    /// Audited re-link of workflow history to a successor element id.
    #[tool(name = "sysml_workflow_relink", description = "Record a deliberate re-link of workflow history from a dead element id to its successor (ADR-009: never automatic). Target must exist in the current workspace; requires an explicit `actor`.")]
    async fn workflow_relink(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workflow.relink", json)
    }

    /// Record a review comment on an element.
    #[tool(name = "sysml_workflow_comment", description = "Record a review comment on an element in the append-only workflow sidecar. The element must exist in the current workspace; requires an explicit non-blank `actor` and `body`.")]
    async fn workflow_comment(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workflow.comment", json)
    }

    /// Assign an engineer to an element.
    #[tool(name = "sysml_workflow_assign", description = "Assign an engineer to an element (workflow sidecar). Folded state keeps the latest assignee; the log keeps every assignment. The element must exist in the current workspace; requires an explicit non-blank `actor` and `assignee`.")]
    async fn workflow_assign(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workflow.assign", json)
    }

    /// Transition an element's approval state.
    #[tool(name = "sysml_workflow_set_approval", description = "Transition an element's approval state (closed vocabulary: draft, in_review, approved, rejected; 'draft' is the initial state). `from` is derived server-side from the folded log — never client-claimed; no-op transitions are rejected. The element must exist; requires an explicit `actor`.")]
    async fn workflow_set_approval(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workflow.set_approval", json)
    }

    /// Record a sign-off attestation statement.
    #[tool(name = "sysml_workflow_sign_off", description = "Record a sign-off attestation statement against an element (workflow sidecar; all sign-offs are kept, oldest-first — a sign-off is a statement of record, never overwritten). The element must exist; requires an explicit non-blank `actor` and `statement`.")]
    async fn workflow_sign_off(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workflow.sign_off", json)
    }

    /// Raw workflow event log for a project (oldest-first).
    #[tool(name = "sysml_workflow_log", description = "The append-only workflow event log for a project, oldest-first; optionally filtered to one element. History keyed on dead ids is still returned, never silently re-attached.")]
    async fn workflow_log(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workflow.log", json)
    }

    /// Folded workflow state of one element.
    #[tool(name = "sysml_workflow_state", description = "Current workflow state of one element derived by folding its event log: approval, assignee, sign-offs, suspect-clearing attestations (flagged superseded when stale), comment count, orphaned status.")]
    async fn workflow_state(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workflow.state", json)
    }

    // -- Parse --

    /// Parse a SysML file and return diagnostics (does not store the graph).
    #[tool(name = "sysml_parse", description = "Parse a SysML file and return the model graph with diagnostics (does not store)")]
    async fn parse(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.parse", json)
    }

    // -- Evaluation --

    /// Evaluate a single element's expression value by its ID.
    #[tool(name = "sysml_evaluate", description = "Evaluate a single element's expression value by its ID")]
    async fn evaluate(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.evaluate", json)
    }

    /// Evaluate one expression-bearing element with optional overrides.
    #[tool(name = "sysml_evaluate_expression", description = "Evaluate one expression-bearing element with optional overrides, returning value, verdict, context, and diagnostics")]
    async fn evaluate_expression(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.evaluate.expression", json)
    }

    /// Evaluate all constraint elements in a model.
    #[tool(name = "sysml_evaluate_constraints", description = "Evaluate all constraint elements in a model, returning pass/fail results with details")]
    async fn evaluate_constraints(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.evaluate.constraints", json)
    }

    /// Evaluate all verification case elements in a model.
    #[tool(name = "sysml_evaluate_verification_cases", description = "Evaluate all verification case elements in a model, returning verdicts per case")]
    async fn evaluate_verification_cases(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.evaluate.verification_cases", json)
    }

    /// Evaluate all analysis case elements in a model.
    #[tool(name = "sysml_evaluate_analysis_cases", description = "Evaluate all analysis case elements in a model, returning output summaries per case")]
    async fn evaluate_analysis_cases(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.evaluate.analysis_cases", json)
    }

    /// Evaluate all calculation elements in a model.
    #[tool(name = "sysml_evaluate_calculations", description = "Evaluate all calculation elements in a model, returning computed values")]
    async fn evaluate_calculations(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.evaluate.calculations", json)
    }

    /// Project expression element subtrees as JSON ASTs for rendering and inspection.
    #[tool(name = "sysml_expression_ast", description = "Project expression element subtrees as JSON ASTs for rendering (KaTeX) and inspection. Pass element_id to project one owner, omit to project all expression-bearing elements.")]
    async fn expression_ast(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.expression.ast", json)
    }

    /// Compile and run a named action to completion.
    #[tool(name = "sysml_action_run", description = "Compile and run a named action to completion, returning the execution trace")]
    async fn action_run(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.action.run", json)
    }

    // -- What-If --

    /// Override a variable value and compare constraint results to find flips.
    #[tool(name = "sysml_whatif", description = "Override a variable value and compare constraint results (baseline vs override) to find flips")]
    async fn whatif(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.whatif", json)
    }

    /// Sweep a parameter across a range and evaluate constraints at each step.
    #[tool(name = "sysml_whatif_sweep", description = "Sweep a parameter across a range and evaluate constraints at each step to find thresholds")]
    async fn whatif_sweep(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.whatif.sweep", json)
    }

    // -- ODE Sweep --

    /// Sweep an ODE parameter across a range, running full simulations per value.
    #[tool(name = "sysml_trade_study_ode_sweep", description = "Sweep an ODE parameter across a range, running full ODE+SM simulations per value to produce a time-current characteristic")]
    async fn trade_study_ode_sweep(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.trade_study.ode_sweep", json)
    }

    // -- Simulation-based verification --

    /// Run ODE simulation with parameter overrides, then evaluate model constraints against the result.
    #[tool(name = "sysml_verify_with_simulation", description = "Run ODE simulation with parameter overrides, then verify the model's verification cases against the result (verdict via VerificationRunner)")]
    async fn verify_with_simulation(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.verify_with_simulation", json)
    }

    /// Run a single simulation scenario and return the full time-series trace for charting.
    #[tool(name = "sysml_verify_with_simulation_trace", description = "Run ODE simulation and return full time-series trace for charting")]
    async fn verify_with_simulation_trace(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.verify_with_simulation_trace", json)
    }

    // -- Aggregation --

    /// Compute aggregate status for all owners in a model.
    #[tool(name = "sysml_aggregate", description = "Compute aggregate constraint/verification/requirement status for all owners in a model")]
    async fn aggregate(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.aggregate", json)
    }

    // -- Workspace verification --

    /// Run cross-file workspace verification.
    #[tool(name = "sysml_workspace_verify", description = "Run cross-file workspace verification, merging all loaded graphs and evaluating all verification cases")]
    async fn workspace_verify(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.verify", json)
    }

    /// Batched tree+stats fetch for every loaded user URI in one round-trip.
    #[tool(name = "sysml_workspace_info", description = "Return tree + stats for every loaded user URI (excludes synthetic __workspace__/__stdlib__) in one call. Optional `uris` array selects a subset.")]
    async fn workspace_info(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.info", json)
    }

    /// Workspace-level lifecycle summary (Bucket A transport-bypass closeout).
    #[tool(name = "sysml_workspace_info_summary", description = "Workspace-level summary: per-root project discovery + loaded host counts + transport-supplied telemetry counters. Takes `workspace_roots: [String]` and `telemetry_counters: [[String, u64]]`.")]
    async fn workspace_info_summary(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.info_summary", json)
    }

    /// List `.sysml`/`.kerml` files under a workspace directory.
    #[tool(name = "sysml_workspace_files", description = "Recursively list .sysml/.kerml files under a workspace directory. Returns a tree pruned to directories that contain such files. Optional `max_depth` (default 5).")]
    async fn workspace_files(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.files", json)
    }

    /// Workspace refresh (S2.T11 — Bucket F / LSP-04, LSP-07, LSP-70).
    #[tool(name = "sysml_workspace_refresh", description = "Rediscover projects across workspace roots, reset the shared host, re-register projects, re-enable stdlib. Returns the discovered project list + stdlib status. Takes `roots: [String]`, optional `enable_stdlib: bool`.")]
    async fn workspace_refresh(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.workspace.refresh", json)
    }

    /// Dependency status (S2.T11 — Bucket F / LSP-63).
    #[tool(name = "sysml_dependency_status", description = "Walk workspace roots, hydrate manifest dependencies, and report per-root resolution outcomes + summary counts. Takes `roots: [String]`.")]
    async fn dependency_status(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.dependency.status", json)
    }

    /// Library cache surface (S2.T11 — Bucket F).
    #[tool(name = "sysml_cache_status", description = "Library cache file snapshot (size_bytes, element_count, crate_version, exists). Returns {status: no_library} when no library is configured.")]
    async fn cache_status(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.cache.status", json)
    }

    /// Delete the library cache file.
    #[tool(name = "sysml_cache_clear", description = "Delete the library cache file from disk; returns {status: cleared|no_library} or {error}.")]
    async fn cache_clear(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.cache.clear", json)
    }

    /// Compute a library cache rebuild payload (clears the cache; reload spawn stays on the LSP transport).
    #[tool(name = "sysml_cache_rebuild", description = "Clear the library cache and return before/after snapshots + library state. The reload spawn is the transport's responsibility (the LSP fires it via tower-lsp progress notifications).")]
    async fn cache_rebuild(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.cache.rebuild", json)
    }

    /// Salsa query telemetry (S2.T11 — Bucket F).
    #[tool(name = "sysml_salsa_stats", description = "Salsa query execution statistics: executions, validations, and approximate cache hit ratio.")]
    async fn salsa_stats(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.salsa.stats", json)
    }

    /// Reset salsa query statistics on the shared host.
    #[tool(name = "sysml_salsa_stats_reset", description = "Reset salsa query execution statistics to zero on the shared host.")]
    async fn salsa_stats_reset(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.salsa.stats.reset", json)
    }

    // -- Breakpoint primitives (R1.2) --

    /// Register a breakpoint on a running session; returns an opaque id.
    #[tool(name = "sysml_breakpoint_set", description = "Register a breakpoint on a running session; returns an opaque BreakpointId. The breakpoint payload is a tagged union: {\"kind\": \"state-entry\"|\"transition-fire\"|\"action-invoke\"|\"constraint-violation\", \"element_id\": \"...\"} or {\"kind\": \"threshold-crossing\", \"variable\": \"...\", \"op\": \"lt\"|\"le\"|\"gt\"|\"ge\"|\"eq\"|\"ne\", \"value\": 42.0, \"debounce_ticks\": 0} or {\"kind\": \"conditional\", \"target\": \"...\", \"variable\": \"...\", \"op\": \"lt\"|\"le\"|\"gt\"|\"ge\"|\"eq\"|\"ne\", \"value\": 42.0, \"enabled\": true, \"label\": \"optional\"}.")]
    async fn breakpoint_set(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.breakpoint.set", json)
    }

    /// Remove a previously-registered breakpoint from a session (idempotent).
    #[tool(name = "sysml_breakpoint_clear", description = "Remove a previously-registered breakpoint from a session by id (idempotent on unknown ids).")]
    async fn breakpoint_clear(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.breakpoint.clear", json)
    }

    /// List registered breakpoints for a session as `(id, breakpoint)` pairs.
    #[tool(name = "sysml_breakpoint_list", description = "List all registered breakpoints on a session as (id, breakpoint) pairs in deterministic id order.")]
    async fn breakpoint_list(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.breakpoint.list", json)
    }

    // -- Batch sessions (R5.0) --

    /// Create a parent BatchSession wrapping N runtime-session children
    /// (sweep / monte_carlo / trade_study).
    #[tool(
        name = "sysml_batch_create",
        description = "Create a BatchSession parenting N child runtime sessions. Required params: kind (\"sweep\"|\"monte_carlo\"|\"trade_study\"), uri, children_params (JSON string: array of per-child override maps e.g. '[{\"mass\": 1.0}, {\"mass\": 2.0}]'). Optional: subsystem_name, label. Returns { batch_id, child_session_ids }."
    )]
    async fn batch_create(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.batch.create", json)
    }

    /// Return the current snapshot of a BatchSession (kind, children,
    /// rollup status).
    #[tool(
        name = "sysml_batch_status",
        description = "Fetch the current BatchSession snapshot by batch_id. Returns { batch: BatchSession } with per-child descriptors and aggregated status."
    )]
    async fn batch_status(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.batch.status", json)
    }

    /// Return every child descriptor for a batch, optionally including
    /// each child's archived verdicts.
    #[tool(
        name = "sysml_batch_results",
        description = "Return per-child descriptors for a batch. Params: batch_id, include_verdicts (bool). When include_verdicts=false the verdicts array is cleared to keep payloads small."
    )]
    async fn batch_results(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.batch.results", json)
    }

    /// Apply a BatchFilter to a batch and return the matching subset.
    ///
    /// Filter shape: `{ only_status?, only_verdict?, param_predicate?: { param, op, value } }`
    /// where `op` ∈ `lt|le|gt|ge|eq|ne`.
    #[tool(
        name = "sysml_batch_slice",
        description = "Filter a batch's children. Params: batch_id, filter (object). filter shape: {\"only_status\": \"pending\"|\"running\"|\"complete\"|\"failed\", \"only_verdict\": \"pass\"|\"fail\"|\"inconclusive\"|\"error\", \"param_predicate\": {\"param\": \"...\", \"op\": \"lt\"|\"le\"|\"gt\"|\"ge\"|\"eq\"|\"ne\", \"value\": f64}}. All fields additive; omitted = no constraint."
    )]
    async fn batch_slice(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.batch.slice", json)
    }

    // -- Sensitivity analysis (R7.4) --

    /// Post-process a completed `kind: "sensitivity"` batch into Morris
    /// or Sobol indices.
    #[tool(
        name = "sysml_sensitivity_analyze",
        description = "Compute per-parameter sensitivity indices from a completed sensitivity batch. Params: batch_id (string), method (\"morris\"|\"sobol\"), parameters_of_interest (JSON string: array of {name,min,max} in the same order the batch was generated), output_metric (string — verdict-derived \"fail_count\"|\"pass_count\"|\"verdict_numeric\" or a key in each child's params map), morris_levels (optional usize, default 4, ignored for Sobol). Returns { method, parameters: [{ name, mu?, sigma?, s1?, st? }] } — Morris populates mu/sigma, Sobol populates s1/st."
    )]
    async fn sensitivity_analyze(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.sensitivity.analyze", json)
    }

    /// Walk backward through the causation graph from a failure event (R7.1).
    /// Either `root_event_id` (preferred) or the `(root_tick, root_target)`
    /// pair identifies the root. `max_depth` defaults to 5 server-side.
    #[tool(
        name = "sysml_causation_trace",
        description = "Walk backward through the causation graph from a failure event. Params: session_id (required), root_event_id OR (root_tick + root_target), max_depth (optional, default 5). Returns { root, chain, max_depth_used }."
    )]
    async fn causation_trace(
        &self,
        params: Parameters<JsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let json = serde_json::to_value(params.0.params).unwrap_or_default();
        dispatch_to_service(&self.service, "sysml.causation.trace", json)
    }
}

#[rmcp::tool_handler]
impl rmcp::ServerHandler for SysmlMcpHandler {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.instructions = Some(
            "SysML v2 model intelligence server. Load models with sysml_load_source, \
             then query with sysml_find, sysml_element, sysml_children, sysml_stats, etc. \
             Tool responses that touch a URI include a `_readiness` envelope field \
             describing library/project/file load state — use it to decide whether to \
             retry empty results while the workspace is still indexing. \
             Progress events are streamed as MCP `notifications/message` log messages \
             (logger=\"sysml_mcp.progress\") whose `data` is the JSON-serialized \
             ProgressEvent (LibraryLoad / WorkspaceIndex / DependencyFetch / Refresh / Ready)."
                .into(),
        );
        info.capabilities.tools = Some(rmcp::model::ToolsCapability { list_changed: None });
        // Advertise the logging capability so notification-aware clients
        // accept our progress stream.
        info.capabilities.logging = Some(serde_json::Map::new());
        info
    }

    /// Once the MCP client finishes initialization, spawn a task that
    /// forwards every [`sysml_service::progress::ProgressEvent`] over
    /// the protocol as a `notifications/message` log entry. The peer
    /// from `context` is the only handle for server-initiated sends.
    ///
    /// We use the logging notification (not progress notifications) so
    /// the events surface in clients that don't pre-register progress
    /// tokens. The JSON payload is the same `ProgressEvent` shape the
    /// REST SSE stream uses, keyed by `kind`.
    fn on_initialized(
        &self,
        context: NotificationContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        async move {
            // Idempotent: a reconnecting client could replay `initialized`.
            if self
                .progress_forwarder_spawned
                .swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                tracing::debug!("progress forwarder already spawned; ignoring re-init");
                return;
            }
            let mut rx = self.service.subscribe_progress();
            let peer = context.peer.clone();
            tokio::spawn(async move {
                use rmcp::model::{LoggingLevel, LoggingMessageNotificationParam};
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            // Translate ProgressEvent kind → MCP log level.
                            let level = match &event {
                                sysml_service::progress::ProgressEvent::LibraryLoad {
                                    phase,
                                    ..
                                } => match phase {
                                    sysml_service::progress::LibraryPhase::Failed => {
                                        LoggingLevel::Error
                                    }
                                    _ => LoggingLevel::Info,
                                },
                                _ => LoggingLevel::Info,
                            };
                            let data = match serde_json::to_value(&event) {
                                Ok(v) => v,
                                Err(e) => {
                                    tracing::warn!(
                                        "failed to serialize progress event: {e}"
                                    );
                                    continue;
                                }
                            };
                            let params = LoggingMessageNotificationParam {
                                level,
                                logger: Some("sysml_mcp.progress".to_owned()),
                                data,
                            };
                            if let Err(e) = peer.notify_logging_message(params).await {
                                // Peer dropped — stop forwarding.
                                tracing::debug!(
                                    "progress forwarder stopping; peer closed: {e}"
                                );
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(
                                "progress forwarder lagged; dropped {n} events"
                            );
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            tracing::debug!(
                                "progress forwarder stopping; service bus closed"
                            );
                            break;
                        }
                    }
                }
            });
            tracing::info!("MCP progress forwarder spawned");
        }
    }
}

/// Start the MCP server on stdio with the given service.
pub async fn serve(service: Arc<SysmlService>) -> Result<(), Box<dyn std::error::Error>> {
    let transport = rmcp::transport::io::stdio();
    let handler = SysmlMcpHandler::new(service);
    let server = handler.serve(transport).await?;
    server.waiting().await?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    /// Verify that every registered service command has a corresponding MCP tool.
    ///
    /// This test ensures 100% coverage: if a new `#[service_command]` is added
    /// to the service layer, this test fails until a matching MCP tool is wired up.
    #[test]
    fn all_commands_have_mcp_tools() {
        // Get all registered command names from the service layer
        let registry_names: std::collections::HashSet<&str> =
            sysml_service::registered_command_metas()
                .iter()
                .map(|m| m.name)
                .collect();

        // Get all MCP tool names from the tool router.
        // MCP tool naming convention: dots replaced with underscores.
        // e.g. service "sysml.evaluate.constraints" -> MCP "sysml_evaluate_constraints"
        let tool_router = SysmlMcpHandler::tool_router();
        let tools = tool_router.list_all();
        let tool_names: std::collections::HashSet<String> = tools
            .iter()
            .map(|t| t.name.to_string())
            .collect();

        // Check every service command has a corresponding MCP tool.
        // To match: convert the service command name's dots to underscores
        // and check if that tool name exists.
        let mut missing: Vec<&str> = Vec::new();
        for name in &registry_names {
            let expected_tool_name = name.replace('.', "_");
            if !tool_names.contains(&expected_tool_name) {
                missing.push(name);
            }
        }

        assert!(
            missing.is_empty(),
            "Service commands missing MCP tools: {:?}\n\
             Registry has {} commands, MCP has {} tools.\n\
             Tool names: {:?}",
            missing,
            registry_names.len(),
            tool_names.len(),
            tool_names,
        );
    }
}
