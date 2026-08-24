//! Command handlers for LSP executeCommand requests.
//!
//! This module extracts all command handling logic from the main server
//! to improve maintainability and reduce the size of the monolithic
//! `execute_command` method.

// LSP server: tower-lsp patterns use unwrap/expect for client sends,
// indexing is bounds-checked by protocol invariants, Arc cloning is intentional.
#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::manual_let_else,
    clippy::arc_with_non_send_sync,
    clippy::clone_on_ref_ptr,
    clippy::map_err_ignore,
    clippy::needless_pass_by_value,
    clippy::panic
)]

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use sysml_ide_db::Cancelled;
use sysml_service::readiness::LibraryReadiness;
use tokio::sync::RwLock;
use tower_lsp::lsp_types::{MessageType, Position};
use tower_lsp::Client;

use crate::diagram;
use crate::telemetry_control;
use crate::telemetry_events;
use crate::utils::position_to_offset;
use crate::ux_messages;
use crate::workspace;

/// Snapshot of library state for the LSP `sysml.*` debug/status JSON
/// payloads. Replaces the retired LSP-side `LibraryState` enum (P-RA4).
/// Combines:
/// - `Readiness.library` for the lifecycle state (Unloaded / Loading /
///   Loaded / Failed) — populated by the service's `ProgressBus`.
/// - `host.library_graph()` for the live graph + element count when
///   loaded.
#[derive(Clone)]
struct LibrarySnapshot {
    state: LibraryReadiness,
    /// Element count when the host has a library graph registered.
    /// `None` for any non-`Loaded` state. (The snapshot deliberately
    /// keeps no graph handle — the last walker, `requirements_trace`,
    /// was deleted under debt-ledger L59, and cloning the whole library
    /// graph per status capture was dead weight.)
    element_count: Option<usize>,
}

impl LibrarySnapshot {
    fn capture(ctx: &CommandContext<'_>) -> Self {
        let state = ctx.service.readiness_for("__library__").library;
        let element_count = {
            let host = ctx.analysis_host.lock().unwrap();
            host.library_graph()
                .map(|lib| lib.data(host.db()).graph().element_count())
        };
        Self {
            state,
            element_count,
        }
    }

    fn name(&self) -> &'static str {
        match &self.state {
            LibraryReadiness::Unloaded => "Unloaded",
            LibraryReadiness::Loading => "Loading",
            LibraryReadiness::Loaded => "Loaded",
            LibraryReadiness::Failed(_) => "Failed",
        }
    }

    fn error(&self) -> Option<String> {
        match &self.state {
            LibraryReadiness::Failed(err) => Some(err.to_string()),
            _ => None,
        }
    }

    fn is_loaded(&self) -> bool {
        matches!(self.state, LibraryReadiness::Loaded)
    }
}

/// Custom notification for sending renderer-neutral ViewModels to the client.
pub(crate) struct DiagramSetViewModelNotification;

impl tower_lsp::lsp_types::notification::Notification for DiagramSetViewModelNotification {
    type Params = diagram::DiagramSetViewModelParams;
    const METHOD: &'static str = diagram::DIAGRAM_SET_VIEW_MODEL_METHOD;
}

/// Context bundle passed to command handlers.
///
/// Provides access to server state and lazy salsa resolution.
/// File resolution happens on-demand via async methods, avoiding
/// the cost of eagerly resolving all open files at dispatch time.
pub(crate) struct CommandContext<'a> {
    pub client: &'a Client,
    pub analysis_host: &'a Arc<std::sync::Mutex<sysml_ide_db::AnalysisHost>>,
    pub service: &'a sysml_service::SysmlService,
    pub workspace_roots: &'a Arc<RwLock<Vec<String>>>,
}

impl<'a> CommandContext<'a> {
    /// Lazily resolve a single file, returning content + graph + position map.
    ///
    /// Returns `Ok(None)` if the file is not tracked, `Err(Cancelled)` if
    /// the query was cancelled by a concurrent edit.
    pub async fn get_resolved(
        &self,
        uri: &str,
    ) -> Result<Option<(String, sysml_core::ModelGraph, sysml_ide_db::PositionMap)>, Cancelled> {
        // Phase 1: Lock -> extract snapshot data -> drop lock.
        let (sf, analysis, project_id) = {
            let host = self.analysis_host.lock().unwrap();
            let Some(file_id) = host.file_id(uri) else {
                return Ok(None);
            };
            let Some(sf) = host.source_file(file_id) else {
                return Ok(None);
            };
            let analysis = host.analysis();
            let project_id = host.find_project_for_uri(uri);
            (sf, analysis, project_id)
        }; // lock dropped here

        // Phase 2: Run expensive resolution on the snapshot (no lock held).
        // Routes through the salsa-tracked `resolve_file_best` dispatcher; the
        // soft library-only fallback is gone, workspace context flows in when
        // the host knows the file's project (P4 will harden the missing-PFS
        // path to fail-hard in the LSP shell).
        let result = Cancelled::catch(std::panic::AssertUnwindSafe(|| {
            let content = analysis.file_text(sf).to_owned();
            let resolved = analysis.resolve_file_best(sf, project_id);
            let graph = resolved.graph().clone();
            let pos_map = analysis.position_map(sf);
            (content, graph, pos_map)
        }))?;
        Ok(Some(result))
    }

    /// Number of tracked files (cheap — just reads FileSet size).
    pub async fn file_count(&self) -> usize {
        self.analysis_host.lock().unwrap().file_count()
    }
}

/// Helper to create an error JSON response.
pub(crate) fn error_json(msg: &str) -> serde_json::Value {
    serde_json::json!({"error": msg})
}

fn require_string_arg(
    args: &[serde_json::Value],
    index: usize,
    name: &str,
) -> Result<String, serde_json::Value> {
    let Some(value) = args.get(index) else {
        return Err(error_json(&format!("missing argument '{name}'")));
    };
    value
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| error_json(&format!("invalid argument '{name}': expected string")))
}

fn optional_string_arg(
    args: &[serde_json::Value],
    index: usize,
    name: &str,
) -> Result<Option<String>, serde_json::Value> {
    let Some(value) = args.get(index) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(|s| Some(s.to_owned()))
        .ok_or_else(|| error_json(&format!("invalid argument '{name}': expected string")))
}

fn require_u32_arg(
    args: &[serde_json::Value],
    index: usize,
    name: &str,
) -> Result<u32, serde_json::Value> {
    let Some(value) = args.get(index) else {
        return Err(error_json(&format!("missing argument '{name}'")));
    };
    let Some(raw) = value.as_u64() else {
        return Err(error_json(&format!(
            "invalid argument '{name}': expected unsigned 32-bit integer"
        )));
    };
    u32::try_from(raw).map_err(|_| {
        error_json(&format!(
            "invalid argument '{name}': expected unsigned 32-bit integer"
        ))
    })
}

fn require_f64_arg(
    args: &[serde_json::Value],
    index: usize,
    name: &str,
) -> Result<f64, serde_json::Value> {
    let Some(value) = args.get(index) else {
        return Err(error_json(&format!("missing argument '{name}'")));
    };
    value
        .as_f64()
        .ok_or_else(|| error_json(&format!("invalid argument '{name}': expected number")))
}

fn require_usize_arg(
    args: &[serde_json::Value],
    index: usize,
    name: &str,
) -> Result<usize, serde_json::Value> {
    let Some(value) = args.get(index) else {
        return Err(error_json(&format!("missing argument '{name}'")));
    };
    let Some(raw) = value.as_u64() else {
        return Err(error_json(&format!(
            "invalid argument '{name}': expected unsigned integer"
        )));
    };
    usize::try_from(raw).map_err(|_| {
        error_json(&format!(
            "invalid argument '{name}': expected unsigned integer"
        ))
    })
}

/// Emit a user-visible command result and mirror it to LSP logs.
async fn show_command_result(ctx: &CommandContext<'_>, message: impl Into<String>) {
    let message = message.into();
    ctx.client
        .show_message(MessageType::INFO, message.clone())
        .await;
    ux_messages::info(ctx.client, message).await;
}

fn lsp_cache_dir() -> PathBuf {
    directories::ProjectDirs::from("rs", "sysml", "sysml-lsp")
        .map(|d| d.cache_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("/tmp/sysml-rs"))
}

fn lsp_panic_log_path() -> PathBuf {
    lsp_cache_dir().join("lsp-panic.log")
}

fn lsp_log_path() -> PathBuf {
    lsp_cache_dir().join("lsp.log")
}

fn whatif_reports_dir() -> PathBuf {
    lsp_cache_dir().join("whatif-reports")
}

fn sanitize_filename_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let out = out.trim_matches('_');
    if out.is_empty() {
        "value".to_owned()
    } else {
        out.to_owned()
    }
}

fn join_preview(items: &[String], limit: usize) -> String {
    if items.is_empty() {
        return "none".to_owned();
    }

    let preview = items
        .iter()
        .take(limit)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if items.len() > limit {
        format!("{preview} (+{} more)", items.len() - limit)
    } else {
        preview
    }
}

fn library_state_snapshot_json(snap: &LibrarySnapshot) -> serde_json::Value {
    serde_json::json!({
        "state": snap.name(),
        "element_count": snap.element_count.map(|c| serde_json::Value::from(c)).unwrap_or(serde_json::Value::Null),
        "error": snap.error().map(serde_json::Value::from).unwrap_or(serde_json::Value::Null),
    })
}

fn read_log_tail(path: &Path, max_bytes: usize, max_lines: usize) -> serde_json::Value {
    let path_str = path.display().to_string();

    let metadata = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return serde_json::json!({
                "path": path_str,
                "exists": false,
                "bytes": 0u64,
                "tail_lines": Vec::<String>::new(),
                "line_count": 0usize,
                "truncated": false,
                "error": serde_json::Value::Null,
            });
        }
        Err(e) => {
            return serde_json::json!({
                "path": path_str,
                "exists": false,
                "bytes": 0u64,
                "tail_lines": Vec::<String>::new(),
                "line_count": 0usize,
                "truncated": false,
                "error": e.to_string(),
            });
        }
    };

    let file_size = metadata.len() as usize;
    let read_len = file_size.min(max_bytes);
    let start = file_size.saturating_sub(read_len) as u64;

    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) => {
            return serde_json::json!({
                "path": path_str,
                "exists": true,
                "bytes": metadata.len(),
                "tail_lines": Vec::<String>::new(),
                "line_count": 0usize,
                "truncated": start > 0,
                "error": e.to_string(),
            });
        }
    };

    if let Err(e) = file.seek(SeekFrom::Start(start)) {
        return serde_json::json!({
            "path": path_str,
            "exists": true,
            "bytes": metadata.len(),
            "tail_lines": Vec::<String>::new(),
            "line_count": 0usize,
            "truncated": start > 0,
            "error": e.to_string(),
        });
    }

    let mut buf = Vec::with_capacity(read_len);
    if let Err(e) = file.read_to_end(&mut buf) {
        return serde_json::json!({
            "path": path_str,
            "exists": true,
            "bytes": metadata.len(),
            "tail_lines": Vec::<String>::new(),
            "line_count": 0usize,
            "truncated": start > 0,
            "error": e.to_string(),
        });
    }

    let mut truncated = start > 0;
    let mut lines: Vec<String> = String::from_utf8_lossy(&buf)
        .lines()
        .map(|line| line.to_owned())
        .collect();
    if lines.len() > max_lines {
        let split_at = lines.len() - max_lines;
        lines = lines.split_off(split_at);
        truncated = true;
    }

    serde_json::json!({
        "path": path_str,
        "exists": true,
        "bytes": metadata.len(),
        "tail_lines": lines.clone(),
        "line_count": lines.len(),
        "truncated": truncated,
        "error": serde_json::Value::Null,
    })
}

fn persist_whatif_report(
    kind: &str,
    uri: &str,
    variable_name: &str,
    summary: &str,
    payload: &serde_json::Value,
) -> serde_json::Value {
    let directory = whatif_reports_dir();
    let directory_display = directory.display().to_string();
    if let Err(err) = std::fs::create_dir_all(&directory) {
        return serde_json::json!({
            "status": "error",
            "directory": directory_display,
            "error": err.to_string(),
        });
    }

    let generated = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let generated_unix = generated.as_secs();
    let generated_nanos = generated.subsec_nanos();

    let file_id = format!(
        "{}-{:09}-{}-{}",
        generated_unix,
        generated_nanos,
        kind,
        sanitize_filename_component(variable_name)
    );
    let json_path = directory.join(format!("{file_id}.json"));
    let markdown_path = directory.join(format!("{file_id}.md"));

    let report = serde_json::json!({
        "report_version": 1u32,
        "kind": kind,
        "generated_unix": generated_unix,
        "uri": uri,
        "variable": variable_name,
        "summary": summary,
        "payload": payload,
    });

    let json_body = match serde_json::to_string_pretty(&report) {
        Ok(body) => body,
        Err(err) => {
            return serde_json::json!({
                "status": "error",
                "directory": directory_display,
                "error": format!("failed to encode report JSON: {err}"),
            });
        }
    };

    if let Err(err) = std::fs::write(&json_path, json_body) {
        return serde_json::json!({
            "status": "error",
            "directory": directory_display,
            "json_path": json_path.display().to_string(),
            "error": format!("failed to write JSON report: {err}"),
        });
    }

    let markdown_body = format!(
        "# SysML What-If Report\n\
\n\
- kind: `{}`\n\
- generated_unix: `{}`\n\
- variable: `{}`\n\
- uri: `{}`\n\
\n\
## Summary\n\
\n\
{}\n\
\n\
## JSON Payload\n\
\n\
```json\n\
{}\n\
```\n",
        kind,
        generated_unix,
        variable_name,
        uri,
        summary,
        serde_json::to_string_pretty(payload).unwrap_or_else(|_| "{}".to_owned())
    );

    if let Err(err) = std::fs::write(&markdown_path, markdown_body) {
        return serde_json::json!({
            "status": "error",
            "directory": directory_display,
            "json_path": json_path.display().to_string(),
            "markdown_path": markdown_path.display().to_string(),
            "error": format!("failed to write Markdown report: {err}"),
        });
    }

    serde_json::json!({
        "status": "written",
        "kind": kind,
        "generated_unix": generated_unix,
        "directory": directory_display,
        "json_path": json_path.display().to_string(),
        "markdown_path": markdown_path.display().to_string(),
    })
}

fn recent_whatif_reports_payload(limit: usize) -> serde_json::Value {
    let directory = whatif_reports_dir();
    let directory_display = directory.display().to_string();
    let read_dir = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return serde_json::json!({
                "directory": directory_display,
                "exists": false,
                "count": 0usize,
                "recent": Vec::<serde_json::Value>::new(),
                "error": serde_json::Value::Null,
            });
        }
        Err(err) => {
            return serde_json::json!({
                "directory": directory_display,
                "exists": false,
                "count": 0usize,
                "recent": Vec::<serde_json::Value>::new(),
                "error": err.to_string(),
            });
        }
    };

    let mut files = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|v| v.to_str()) != Some("json") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        files.push((path, metadata.len(), modified));
    }

    files.sort_by(|a, b| b.2.cmp(&a.2));

    let total_count = files.len();
    let recent = files
        .into_iter()
        .take(limit)
        .map(|(json_path, bytes, modified)| {
            let modified_unix = modified
                .duration_since(SystemTime::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_secs());
            let markdown_path = json_path.with_extension("md");

            let (kind, variable, summary) = std::fs::read_to_string(&json_path)
                .ok()
                .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
                .map(|report| {
                    (
                        report
                            .get("kind")
                            .and_then(|v| v.as_str())
                            .map(|v| v.to_owned()),
                        report
                            .get("variable")
                            .and_then(|v| v.as_str())
                            .map(|v| v.to_owned()),
                        report
                            .get("summary")
                            .and_then(|v| v.as_str())
                            .map(|v| v.to_owned()),
                    )
                })
                .unwrap_or((None, None, None));

            serde_json::json!({
                "json_path": json_path.display().to_string(),
                "markdown_path": markdown_path.display().to_string(),
                "bytes": bytes,
                "modified_unix": modified_unix,
                "kind": kind,
                "variable": variable,
                "summary": summary,
                "json_tail": read_log_tail(&json_path, 64 * 1024, 40),
                "markdown_tail": read_log_tail(&markdown_path, 32 * 1024, 30),
            })
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "directory": directory_display,
        "exists": true,
        "count": total_count,
        "recent": recent,
        "error": serde_json::Value::Null,
    })
}

async fn debug_status_payload(ctx: &CommandContext<'_>) -> serde_json::Value {
    let library_snap = LibrarySnapshot::capture(ctx);
    let configured_library_path =
        workspace::find_library_config().map(|config| config.library_path.display().to_string());

    let library_state_name = library_snap.name();
    let library_error = library_snap.error();
    let library_elements = library_snap.element_count;

    // Count documents using the salsa graph cache (populated at command dispatch time).
    // All files in the cache have been resolved; tier depends on library state.
    let open_documents = ctx.file_count().await;
    let parsed_documents = open_documents; // all salsa files are parsed
    let has_library = library_snap.is_loaded();
    let t1_syntax_docs = 0usize;
    let t2_local_docs = if has_library { 0 } else { open_documents };
    let t3_full_docs = if has_library { open_documents } else { 0 };

    let panic_log_path = lsp_panic_log_path();
    let panic_meta = std::fs::metadata(&panic_log_path).ok();
    let panic_log_exists = panic_meta.is_some();
    let panic_log_bytes = panic_meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let panic_log_modified_unix = panic_meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    let recent_panic = panic_meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .map(|age| age <= Duration::from_secs(48 * 60 * 60))
        .unwrap_or(false);

    let lsp_log_file = lsp_log_path();
    let lsp_log_exists = std::fs::metadata(&lsp_log_file).is_ok();

    let (health, reason) = if recent_panic {
        ("broken", "recent_panic_log")
    } else {
        match &library_snap.state {
            LibraryReadiness::Failed(_) => ("broken", "library_failed"),
            LibraryReadiness::Unloaded => ("degraded", "library_unloaded"),
            LibraryReadiness::Loading => ("degraded", "library_loading"),
            LibraryReadiness::Loaded => {
                if t1_syntax_docs > 0 {
                    ("degraded", "syntax_only_documents")
                } else {
                    ("healthy", "ok")
                }
            }
        }
    };

    serde_json::json!({
        "health": health,
        "reason": reason,
        "library": {
            "state": library_state_name,
            "error": library_error,
            "element_count": library_elements,
            "configured_path": configured_library_path,
        },
        "documents": {
            "open": open_documents,
            "parsed": parsed_documents,
            "tiers": {
                "t1_syntax": t1_syntax_docs,
                "t2_local": t2_local_docs,
                "t3_full": t3_full_docs,
            }
        },
        "panic": {
            "log_exists": panic_log_exists,
            "recent": recent_panic,
            "path": panic_log_path.display().to_string(),
            "bytes": panic_log_bytes,
            "modified_unix": panic_log_modified_unix,
        },
        "logs": {
            "lsp_log_path": lsp_log_file.display().to_string(),
            "lsp_log_exists": lsp_log_exists,
        },
        "commands": {
            "status": "sysml.debug.status",
            "bundle": "sysml.debug.bundle",
            "cache_status": "sysml.cache.status",
            "cache_rebuild": "sysml.cache.rebuild",
        }
    })
}

// ============================================================================
// Cache commands
// ============================================================================

pub(crate) async fn handle_cache_clear(ctx: &CommandContext<'_>) -> serde_json::Value {
    let payload = match ctx.service.cache_clear() {
        Ok(v) => v,
        Err(e) => return error_json(&format!("cache.clear failed: {e}")),
    };
    match payload.get("status").and_then(|v| v.as_str()) {
        Some("cleared") => {
            ctx.client
                .show_message(MessageType::INFO, "SysML library cache cleared")
                .await;
        }
        Some("no_library") => {
            ctx.client
                .show_message(MessageType::WARNING, "No library configured")
                .await;
        }
        _ => {
            if let Some(err) = payload.get("error").and_then(|v| v.as_str()) {
                ctx.client
                    .show_message(MessageType::ERROR, format!("Failed to clear cache: {}", err))
                    .await;
            }
        }
    }
    payload
}

pub(crate) async fn handle_cache_status(ctx: &CommandContext<'_>) -> serde_json::Value {
    let snapshot = match ctx.service.cache_status() {
        Ok(v) => v,
        Err(e) => return error_json(&format!("cache.status failed: {e}")),
    };
    if snapshot
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| s == "no_library")
        .unwrap_or(false)
    {
        ux_messages::show_warn(ctx.client, "No library configured").await;
    } else if snapshot
        .get("exists")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let element_count = snapshot
            .get("element_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let size_bytes = snapshot
            .get("size_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let crate_version = snapshot
            .get("crate_version")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let msg = format!(
            "Cache: {} elements, {:.1} KB, version {}",
            element_count,
            size_bytes as f64 / 1024.0,
            crate_version
        );
        ux_messages::show_info(ctx.client, msg).await;
    } else {
        ux_messages::show_info(ctx.client, "No cache file found").await;
    }
    snapshot
}

pub(crate) async fn handle_cache_rebuild<F>(
    ctx: &CommandContext<'_>,
    task_id: String,
    reload_callback: F,
) -> serde_json::Value
where
    F: FnOnce() + Send + 'static,
{
    // Capture the LSP-side `LibraryState` enum BEFORE resetting it. The
    // service-side `cache_rebuild` returns a coarse `Loaded|Unloaded`
    // derived from `host.library_graph()`, but the LSP shell exposes the
    // richer `Loading`/`Failed` variants too — so we override the
    // `library_before` field with the LSP shape to preserve byte identity
    // for the LSP transport. Cross-transport callers (CLI/MCP/REST) get
    // the simpler service shape, which is the new advertised contract.
    let library_before = library_state_snapshot_json(&LibrarySnapshot::capture(ctx));

    // Service does the heavy lifting: cache stats before/after, clear the
    // file, derive `clear_status`, build the JSON payload.
    let mut payload = match ctx.service.cache_rebuild() {
        Ok(v) => v,
        Err(e) => return error_json(&format!("cache.rebuild failed: {e}")),
    };
    let clear_status = payload
        .get("clear_status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_owned();

    // Override library_before with the LSP-side rich shape.
    if let Some(map) = payload.as_object_mut() {
        map.insert("library_before".to_owned(), library_before.clone());
    }

    // Reset service-tracked library lifecycle so the spawned reload
    // sees `Unloaded` from `readiness_for` and proceeds with the load
    // pipeline (P-RA4).
    ctx.service.reset_library_lifecycle();

    // Spawn the reload task — UX progress notifications are tower-lsp
    // protocol concerns and stay on the LSP shell.
    tokio::spawn(async move {
        tracing::info!(
            task_id = %task_id,
            task = "cache_rebuild_dispatch",
            "running cache rebuild reload callback"
        );
        reload_callback();
    });

    let summary = format!(
        "Rebuilding library cache (clear status: {}, previous library state: {})",
        clear_status,
        library_before
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
    );
    ux_messages::show_info(ctx.client, summary).await;
    payload
}

/// Handle `sysml.salsa.stats` — return salsa query execution statistics.
pub(crate) async fn handle_salsa_stats(ctx: &CommandContext<'_>) -> serde_json::Value {
    let stats = match ctx.service.salsa_stats() {
        Ok(s) => s,
        Err(e) => return error_json(&format!("salsa.stats failed: {e}")),
    };
    let msg = format!(
        "Salsa: {} executions, {} validations, {:.1}% hit ratio",
        stats.executions,
        stats.validations,
        stats.hit_ratio * 100.0
    );
    ux_messages::show_info(ctx.client, msg).await;
    serde_json::to_value(stats).unwrap_or(serde_json::Value::Null)
}

/// Handle `sysml.salsa.stats.reset` — reset salsa query statistics to zero.
pub(crate) async fn handle_salsa_stats_reset(ctx: &CommandContext<'_>) -> serde_json::Value {
    let result = match ctx.service.salsa_stats_reset() {
        Ok(r) => r,
        Err(e) => return error_json(&format!("salsa.stats.reset failed: {e}")),
    };
    ux_messages::show_info(ctx.client, "Salsa query statistics reset").await;
    serde_json::to_value(result).unwrap_or(serde_json::Value::Null)
}

pub(crate) async fn handle_debug_status(ctx: &CommandContext<'_>) -> serde_json::Value {
    let payload = debug_status_payload(ctx).await;
    let health = payload
        .get("health")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let reason = payload
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    show_command_result(ctx, format!("SysML debug status: {} ({})", health, reason)).await;
    payload
}

pub(crate) async fn handle_debug_bundle(ctx: &CommandContext<'_>) -> serde_json::Value {
    let status = debug_status_payload(ctx).await;
    let health = status
        .get("health")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let reason = status
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let configured_library_path =
        workspace::find_library_config().map(|config| config.library_path.display().to_string());
    let clear_would_run = configured_library_path.is_some();
    let cache_snapshot = ctx
        .service
        .cache_status()
        .unwrap_or_else(|_| serde_json::json!({"status": "no_library"}));
    let library_state = library_state_snapshot_json(&LibrarySnapshot::capture(ctx));
    let lsp_log = lsp_log_path();
    let panic_log = lsp_panic_log_path();
    let (lsp_log_tail, panic_log_tail, whatif_reports) = tokio::task::spawn_blocking(move || {
        let lsp = read_log_tail(&lsp_log, 256 * 1024, 200);
        let panic = read_log_tail(&panic_log, 128 * 1024, 200);
        let reports = recent_whatif_reports_payload(5);
        (lsp, panic, reports)
    })
    .await
    .unwrap_or_else(|e| {
        tracing::error!("debug bundle blocking task panicked: {e}");
        let err = serde_json::json!({"error": "task panicked"});
        (err.clone(), err.clone(), err)
    });
    let generated_unix = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let payload = serde_json::json!({
        "bundle_version": 1u32,
        "generated_unix": generated_unix,
        "status": status,
        "cache": {
            "configured_library_path": configured_library_path,
            "snapshot": cache_snapshot,
            "library_state": library_state,
            "rebuild_preview": {
                "command": "sysml.cache.rebuild",
                "clear_would_run": clear_would_run,
            }
        },
        "logs": {
            "lsp": lsp_log_tail,
            "panic": panic_log_tail,
        },
        "whatif_reports": whatif_reports,
        "commands": {
            "status": "sysml.debug.status",
            "bundle": "sysml.debug.bundle",
            "cache_status": "sysml.cache.status",
            "cache_rebuild": "sysml.cache.rebuild",
        }
    });

    show_command_result(
        ctx,
        format!(
            "SysML debug bundle captured: {} ({}) - includes status + log tails + what-if reports",
            health, reason
        ),
    )
    .await;
    payload
}

// ============================================================================
// Evaluation commands
// ============================================================================

pub(crate) async fn handle_evaluate(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.len() < 3 {
        return error_json("expected 3 arguments: uri, line, character");
    }

    let uri_str = match require_string_arg(args, 0, "uri") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let line = match require_u32_arg(args, 1, "line") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let character = match require_u32_arg(args, 2, "character") {
        Ok(value) => value,
        Err(error) => return error,
    };

    let (content, _graph, position_map) = match ctx.get_resolved(&uri_str).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            return error_json("document not in a workspace; call load_workspace first")
        }
        Err(_cancelled) => return error_json("request cancelled"),
    };

    let offset = position_to_offset(&Position { line, character }, &content);

    let Some(element_id) = position_map.element_id_at(offset) else {
        return error_json("no element at position");
    };

    match ctx.service.evaluate_element(&element_id) {
        Ok(Some(result)) => {
            let msg = format!("Evaluation: {}", result);
            show_command_result(ctx, msg).await;
            serde_json::json!({"result": result})
        }
        Ok(None) => error_json("element not evaluable"),
        Err(e) => error_json(&format!("evaluation failed: {e}")),
    }
}

pub(crate) async fn handle_evaluate_all(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.is_empty() {
        return error_json("expected 1 argument: uri");
    }

    let uri_str = match require_string_arg(args, 0, "uri") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let start = std::time::Instant::now();

    // Route both eval passes through the service. The service evaluates
    // against its workspace-aware graph (cross-file imports resolved) and
    // already returns per-row name/kind/expr enrichment (B8) — the LSP just
    // reshapes the rows into the {name, kind, expression, result, status}
    // wire format consumed by the VS Code extension. Q-LSP-1: dispatch-only,
    // no graph re-walk.
    let mut expressions = Vec::new();

    let constraints_json = match ctx.service.evaluate_constraints() {
        Ok(v) => v,
        Err(e) => return error_json(&format!("evaluate.constraints failed: {e}")),
    };
    if let Some(arr) = constraints_json.as_array() {
        for cr in arr {
            let element_id_str = cr.get("element_id").and_then(|v| v.as_str()).unwrap_or("");
            let satisfied = cr.get("satisfied").and_then(|v| v.as_bool()).unwrap_or(false);
            let detail = cr.get("detail").and_then(|v| v.as_str()).unwrap_or("");
            let display = cr.get("display").and_then(|v| v.as_str()).unwrap_or("");
            // B8: service supplies name/kind/expr; LSP no longer re-walks the
            // graph. Fallbacks here are just defensive in case an older
            // service ever returns a row without the new fields.
            let name = cr
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("<unnamed>")
                .to_owned();
            let kind = cr
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("ConstraintUsage")
                .to_owned();
            let status = if satisfied { "pass" } else { "fail" };
            expressions.push(serde_json::json!({
                "name": name,
                "kind": kind,
                "expression": detail,
                "result": display,
                "status": status,
                "element_id": element_id_str,
            }));
        }
    }

    let calcs_json = match ctx.service.evaluate_calculations() {
        Ok(v) => v,
        Err(e) => return error_json(&format!("evaluate.calculations failed: {e}")),
    };
    if let Some(arr) = calcs_json.as_array() {
        for cr in arr {
            let element_id_str = cr.get("element_id").and_then(|v| v.as_str()).unwrap_or("");
            let display = cr.get("display").and_then(|v| v.as_str()).unwrap_or("");
            // B8: name/kind/expr supplied by the service.
            let name = cr
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("<unnamed>")
                .to_owned();
            let kind = cr
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("CalculationUsage")
                .to_owned();
            let expr = cr
                .get("expr")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            expressions.push(serde_json::json!({
                "name": name,
                "kind": kind,
                "expression": expr,
                "result": display,
                "status": "value",
                "element_id": element_id_str,
            }));
        }
    }

    let elapsed = start.elapsed();
    show_command_result(
        ctx,
        format!(
            "Evaluated {} expressions in {:?}",
            expressions.len(),
            elapsed
        ),
    )
    .await;

    serde_json::json!({
        "uri": uri_str,
        "expressions": expressions,
        "elapsed_ms": elapsed.as_millis(),
    })
}

// ============================================================================
// Verification commands
// ============================================================================

pub(crate) async fn handle_verify(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.len() < 2 {
        return error_json("expected 2 arguments: uri, case_name");
    }

    // Positional `uri` stays on the wire for client arity compat
    // (validated, unused): the service call is workspace-scoped since the
    // workspace-scope collapse (2026-07-16).
    let _uri_str = match require_string_arg(args, 0, "uri") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let case_name = match require_string_arg(args, 1, "case_name") {
        Ok(value) => value,
        Err(error) => return error,
    };

    match ctx.service.verify(&case_name, &[]) {
        Ok(result) => {
            let total = result.requirements.len();
            let passed = result.summary.pass;

            for req in &result.requirements {
                tracing::debug!(
                    case_name = %case_name,
                    requirement_id = %req.requirement_id,
                    verdict = %req.verdict,
                    message = %req.message,
                    "verification requirement result"
                );
            }

            let msg = format!(
                "Verification '{}': {} ({}/{} passed)",
                case_name, result.verdict, passed, total
            );
            tracing::info!(
                case_name = %case_name,
                verdict = %result.verdict,
                passed,
                total,
                "verification complete"
            );
            show_command_result(ctx, msg).await;
            telemetry_events::verify_complete(&case_name, &result.verdict, passed, total);

            serde_json::json!({
                "verdict": result.verdict,
                "total_requirements": total,
                "passed_requirements": passed,
                "case_name": case_name,
            })
        }
        Err(e) => error_json(&e.to_string()),
    }
}

// ============================================================================
// Analysis commands
// ============================================================================

pub(crate) async fn handle_analysis_run(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.len() < 2 {
        return error_json("expected 2 arguments: uri, case_name");
    }
    let uri_str = match require_string_arg(args, 0, "uri") {
        Ok(v) => v,
        Err(e) => return e,
    };
    let case_name = match require_string_arg(args, 1, "case_name") {
        Ok(v) => v,
        Err(e) => return e,
    };

    // Extract optional overrides from args[2] (JSON object of key: value)
    // and lower to (key, string-formatted value) pairs for service dispatch.
    let mut overrides: Vec<(String, String)> = Vec::new();
    if let Some(obj) = args.get(2).and_then(|v| v.as_object()) {
        for (key, val) in obj {
            let s = match val {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::String(s) => s.clone(),
                _ => continue,
            };
            overrides.push((key.clone(), s));
        }
    }

    let start = std::time::Instant::now();

    let result = match ctx.service.analysis_run(&case_name, &overrides) {
        Ok(r) => r,
        Err(e) => return error_json(&format!("analysis failed: {e}")),
    };

    let elapsed = start.elapsed();

    let outputs: serde_json::Map<String, serde_json::Value> = result
        .outputs
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect();

    let msg = format!(
        "Analysis '{}': {} outputs, {} iterations, {}",
        case_name,
        outputs.len(),
        result
            .iterations
            .map(|i| i.to_string())
            .unwrap_or("?".into()),
        if result.converged {
            "CONVERGED"
        } else {
            "NOT CONVERGED"
        },
    );
    show_command_result(ctx, msg).await;

    serde_json::json!({
        "uri": uri_str,
        "case_name": case_name,
        "outputs": outputs,
        "converged": result.converged,
        "iterations": result.iterations,
        "elapsed_ms": elapsed.as_millis(),
        "tool_name": result.tool_name,
    })
}

pub(crate) async fn handle_workspace_verify(ctx: &CommandContext<'_>) -> serde_json::Value {
    let json = match ctx.service.workspace_verify(Some(10)) {
        Ok(j) => j,
        Err(e) => return error_json(&format!("workspace verify failed: {e}")),
    };

    let total_cases = json.get("total_cases").and_then(|v| v.as_u64()).unwrap_or(0);
    let passed = json.get("passed").and_then(|v| v.as_u64()).unwrap_or(0);
    let elapsed_ms = json.get("elapsed_ms").and_then(|v| v.as_u64()).unwrap_or(0);
    let msg = format!(
        "Workspace verification: {}/{} passed ({} cases, {}ms)",
        passed, total_cases, total_cases, elapsed_ms,
    );
    show_command_result(ctx, &msg).await;
    tracing::info!(
        passed,
        total_cases,
        elapsed_ms,
        "workspace verification complete"
    );

    json
}

// ============================================================================
// Requirements traceability
// ============================================================================

pub(crate) async fn handle_workspace_info(ctx: &CommandContext<'_>) -> serde_json::Value {
    let workspace_roots = ctx.workspace_roots.read().await.clone();
    let counters = telemetry_control::counter_snapshot(Some("lsp.counter."));
    let summary = match ctx.service.workspace_info_summary(&workspace_roots, &counters) {
        Ok(s) => s,
        Err(e) => return error_json(&format!("workspace info failed: {e}")),
    };
    let payload = serde_json::to_value(&summary).unwrap_or(serde_json::Value::Null);

    show_command_result(
        ctx,
        format!(
            "Workspace info: {} root(s), {} loaded project(s), {} tracked file(s)",
            summary.workspace_roots.len(),
            summary.loaded.user_projects,
            summary.loaded.tracked_files,
        ),
    )
    .await;
    payload
}

pub(crate) async fn handle_project_info(ctx: &CommandContext<'_>) -> serde_json::Value {
    handle_workspace_info(ctx).await
}

pub(crate) async fn handle_dependency_status(ctx: &CommandContext<'_>) -> serde_json::Value {
    let roots = ctx.workspace_roots.read().await.clone();
    let payload = match ctx.service.dependency_status(&roots) {
        Ok(v) => v,
        Err(e) => return error_json(&format!("dependency.status failed: {e}")),
    };
    let summary = payload.get("summary");
    let total_dependencies = summary
        .and_then(|s| s.get("total_dependencies"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let hydrated_dependencies = summary
        .and_then(|s| s.get("hydrated_dependencies"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let failed_dependencies = summary
        .and_then(|s| s.get("failed_dependencies"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    show_command_result(
        ctx,
        format!(
            "Dependency status: {} root(s), {} dependencies, {} hydrated, {} failed",
            roots.len(),
            total_dependencies,
            hydrated_dependencies,
            failed_dependencies
        ),
    )
    .await;
    payload
}

// ============================================================================
// Simulation commands
// ============================================================================

pub(crate) async fn handle_simulate_start(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.len() < 2 {
        return error_json("expected 2 arguments: uri, sm_name");
    }

    let uri_str = match require_string_arg(args, 0, "uri") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let sm_name = match require_string_arg(args, 1, "sm_name") {
        Ok(value) => value,
        Err(error) => return error,
    };

    // Service does the canonical compile (workspace_aware_graph,
    // ModelCompiler, cap_check, RuntimeSession::new) and inserts the session.
    // It also performs the first step internally, so the returned state
    // reflects entry actions.
    let (session_key, step_result) = match ctx.service.simulate_start(&uri_str, &sm_name) {
        Ok(v) => v,
        Err(e) => return error_json(&e.to_string()),
    };

    let available: Vec<_> = step_result
        .available_transitions
        .iter()
        .map(|(event, target)| serde_json::json!({"event": event, "target": target}))
        .collect();

    show_command_result(
        ctx,
        format!(
            "Simulation '{}' started in state '{}'",
            sm_name, step_result.state
        ),
    )
    .await;

    serde_json::json!({
        "session_id": session_key,
        "state": step_result.state,
        "completed": step_result.completed,
        "available_transitions": available,
    })
}

/// Unified session step — delegates to `service.sessions_step()`.
///
/// Returns `SessionSummary` as JSON (the canonical shape from the session
/// backend contract). All per-kind step handlers delegate here.
pub(crate) async fn handle_sessions_step(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    let session_id = match require_string_arg(args, 0, "session_id") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let event = match optional_string_arg(args, 1, "event") {
        Ok(value) => value,
        Err(error) => return error,
    };
    // Optional bulk-step count (arg 2); absent/non-numeric = single tick.
    let ticks = args.get(2).and_then(|v| v.as_u64());

    match ctx.service.sessions_step(&session_id, event.as_deref(), None, ticks) {
        Ok(summary) => {
            show_command_result(
                ctx,
                format!(
                    "Session step: tick={}, state={:?}, completed={}",
                    summary.tick,
                    summary.current_state.as_deref().unwrap_or("-"),
                    summary.completed
                ),
            )
            .await;
            serde_json::to_value(&summary).unwrap_or_else(|e| error_json(&e.to_string()))
        }
        Err(e) => error_json(&e.to_string()),
    }
}

/// Unified session inject — delegates to `service.sessions_inject()`.
pub(crate) async fn handle_sessions_inject(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    let session_id = match require_string_arg(args, 0, "session_id") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let subsystem = match require_string_arg(args, 1, "subsystem") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let event = match require_string_arg(args, 2, "event") {
        Ok(value) => value,
        Err(error) => return error,
    };

    match ctx.service.sessions_inject(&session_id, &subsystem, &event, None) {
        Ok(summary) => {
            serde_json::to_value(&summary).unwrap_or_else(|e| error_json(&e.to_string()))
        }
        Err(e) => error_json(&e.to_string()),
    }
}

/// Legacy simulate.step — thin wrapper around unified sessions_step.
pub(crate) async fn handle_simulate_step(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    handle_sessions_step(ctx, args).await
}

pub(crate) async fn handle_simulate_stop(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.is_empty() {
        return error_json("expected argument: session_id");
    }

    let session_id = match require_string_arg(args, 0, "session_id") {
        Ok(value) => value,
        Err(error) => return error,
    };
    match ctx.service.simulate_stop(&session_id) {
        Ok(()) => {
            show_command_result(ctx, format!("Simulation '{}' stopped", session_id)).await;
            serde_json::json!({"status": "stopped"})
        }
        Err(_) => error_json("session not found"),
    }
}

pub(crate) async fn handle_simulate_reset(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.is_empty() {
        return error_json("expected argument: session_id");
    }

    let session_id = match require_string_arg(args, 0, "session_id") {
        Ok(value) => value,
        Err(error) => return error,
    };
    match ctx.service.sessions_reset(&session_id) {
        Ok(summary) => {
            let state = summary.current_state.clone().unwrap_or_default();
            show_command_result(
                ctx,
                format!("Simulation '{}' reset to '{}'", session_id, state),
            )
            .await;
            serde_json::json!({
                "state": state,
                "completed": summary.completed,
            })
        }
        Err(_) => error_json("session not found"),
    }
}

// ============================================================================
// Action commands
// ============================================================================

pub(crate) async fn handle_action_run(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.len() < 2 {
        return error_json("expected arguments: uri, action_name");
    }

    let uri = match require_string_arg(args, 0, "uri") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let action_name = match require_string_arg(args, 1, "action_name") {
        Ok(value) => value,
        Err(error) => return error,
    };

    let result = match ctx.service.action_run(&uri, &action_name) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(action = %action_name, error = %e, "action run failed");
            return error_json(&e.to_string());
        }
    };

    let completed = result
        .get("completed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let total_steps = result
        .get("total_steps")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let mut output_idx = 0usize;
    let mut suppressed_outputs = 0usize;
    if let Some(steps) = result.get("steps").and_then(|v| v.as_array()) {
        for (i, step) in steps.iter().enumerate() {
            if let Some(outputs) = step.get("outputs").and_then(|o| o.as_array()) {
                for output in outputs {
                    output_idx += 1;
                    if output_idx <= 10 || output_idx.is_multiple_of(50) {
                        tracing::debug!(
                            action = %action_name,
                            step = i + 1,
                            output_index = output_idx,
                            output = %output,
                            "action output"
                        );
                    } else {
                        suppressed_outputs += 1;
                    }
                }
            }
        }
    }
    if suppressed_outputs > 0 {
        tracing::debug!(
            action = %action_name,
            suppressed_outputs,
            total_outputs = output_idx,
            "suppressed repetitive action output logs"
        );
    }

    show_command_result(
        ctx,
        format!(
            "Action '{}' completed={} steps={} outputs={}",
            action_name, completed, total_steps, output_idx
        ),
    )
    .await;

    serde_json::json!({
        "action": action_name.clone(),
        "completed": completed,
        "totalSteps": total_steps,
    })
}

pub(crate) async fn handle_action_start(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.len() < 2 {
        return error_json("expected 2 arguments: uri, action_name");
    }

    let uri_str = match require_string_arg(args, 0, "uri") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let action_name = match require_string_arg(args, 1, "action_name") {
        Ok(value) => value,
        Err(error) => return error,
    };

    // Service does the canonical compile (workspace_aware_graph,
    // ModelCompiler::compile_action, cap_check, RuntimeSession::new) and
    // inserts the session.
    let session_key = match ctx.service.action_start(&uri_str, &action_name) {
        Ok(k) => k,
        Err(e) => return error_json(&e.to_string()),
    };

    show_command_result(ctx, format!("Action session '{}' started", action_name)).await;

    serde_json::json!({
        "session_id": session_key,
        "completed": false,
    })
}

/// Legacy action.step — thin wrapper around unified sessions_step.
pub(crate) async fn handle_action_step(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    handle_sessions_step(ctx, args).await
}

pub(crate) async fn handle_action_stop(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.is_empty() {
        return error_json("expected argument: session_id");
    }

    let session_id = match require_string_arg(args, 0, "session_id") {
        Ok(value) => value,
        Err(error) => return error,
    };
    // Fetch step count before stopping (sessions_stop returns ()).
    let step_count = ctx
        .service
        .sessions()
        .get(&sysml_service::ElementId::from_string(&session_id))
        .map(|s| s.history().len())
        .unwrap_or(0);
    match ctx.service.sessions_stop(&session_id) {
        Ok(()) => {
            tracing::debug!(session_id = %session_id, steps = step_count, "action session stopped");
            show_command_result(
                ctx,
                format!("Action session '{}' stopped ({} steps)", session_id, step_count),
            )
            .await;
            serde_json::json!({"status": "stopped", "steps": step_count})
        }
        Err(_) => error_json("session not found"),
    }
}

pub(crate) async fn handle_action_reset(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.is_empty() {
        return error_json("expected argument: session_id");
    }

    let session_id = match require_string_arg(args, 0, "session_id") {
        Ok(value) => value,
        Err(error) => return error,
    };
    match ctx.service.sessions_reset(&session_id) {
        Ok(summary) => {
            show_command_result(ctx, format!("Action session '{}' reset", session_id)).await;
            serde_json::json!({"status": "reset", "completed": summary.completed})
        }
        Err(_) => error_json("session not found"),
    }
}

pub(crate) async fn handle_action_visualize(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.len() < 2 {
        return error_json("expected 2 arguments: uri, action_name");
    }

    let uri_str = match require_string_arg(args, 0, "uri") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let action_name = match require_string_arg(args, 1, "action_name") {
        Ok(value) => value,
        Err(error) => return error,
    };

    // The service owns PlantUML projection; the LSP returns its result directly.
    let payload = match ctx.service.action_visualize(&action_name) {
        Ok(v) => v,
        Err(e) => return error_json(&e.to_string()),
    };
    show_command_result(ctx, format!("Action '{}' diagram generated", action_name)).await;
    payload
}

// ============================================================================
// Flow visualization commands
// ============================================================================

pub(crate) async fn handle_flow_visualize(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.is_empty() {
        return error_json("expected argument: uri");
    }

    let uri_str = match require_string_arg(args, 0, "uri") {
        Ok(value) => value,
        Err(error) => return error,
    };

    // The service produces PlantUML; the LSP returns that payload directly.
    let payload = match ctx.service.flow_visualize(None) {
        Ok(v) => v,
        Err(e) => return error_json(&e.to_string()),
    };

    show_command_result(ctx, format!("Flow visualization sent for '{}'", uri_str)).await;
    payload
}

// ============================================================================
// What-if analysis commands
// ============================================================================

pub(crate) async fn handle_whatif(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.len() < 5 {
        return error_json(
            "expected 5 arguments: uri, line, character, variable_name, override_value",
        );
    }

    let uri_str = match require_string_arg(args, 0, "uri") {
        Ok(value) => value,
        Err(error) => return error,
    };
    // line/character are accepted for backward compatibility with code-lens
    // callers but no longer used — the service whatif is whole-graph and
    // takes the variable name directly.
    let _line = match require_u32_arg(args, 1, "line") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let _character = match require_u32_arg(args, 2, "character") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let variable_name = match require_string_arg(args, 3, "variable_name") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let override_json = &args[4];

    // Ensure document is in a workspace before delegating (fail-hard parity
    // with the previous get_resolved guard).
    match ctx.get_resolved(&uri_str).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return error_json("document not in a workspace; call load_workspace first")
        }
        Err(_cancelled) => return error_json("request cancelled"),
    };

    // Service signature accepts override as a string parsed via
    // `parse_value_string`. JSON strings unwrap to bare text; numbers/bools
    // stringify (`1.5`, `true`).
    let override_str = if let Some(s) = override_json.as_str() {
        s.to_owned()
    } else {
        override_json.to_string()
    };
    let override_display = override_str.clone();

    let result_json = match ctx.service.whatif(
        &variable_name,
        &override_str,
        None,
    ) {
        Ok(v) => v,
        Err(e) => return error_json(&format!("whatif failed: {e}")),
    };

    let baseline = result_json
        .get("baseline")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let overridden = result_json
        .get("overridden")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let flipped = result_json
        .get("flipped")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let satisfied_pass = |arr: &[serde_json::Value]| -> usize {
        arr.iter()
            .filter(|item| {
                item.get("satisfied")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false)
            })
            .count()
    };
    let baseline_pass = satisfied_pass(&baseline);
    let override_pass = satisfied_pass(&overridden);
    let total_constraints = baseline.len().max(overridden.len());

    let flipped_name = |item: &serde_json::Value| -> String {
        item.get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("unnamed")
            .to_owned()
    };
    let flipped_to_fail: Vec<String> = flipped
        .iter()
        .filter(|f| {
            !f.get("now_passing")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .map(flipped_name)
        .collect();
    let flipped_to_pass: Vec<String> = flipped
        .iter()
        .filter(|f| {
            f.get("now_passing")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .map(flipped_name)
        .collect();

    let details = if flipped.is_empty() {
        "no flips".to_owned()
    } else {
        let mut parts = Vec::new();
        if !flipped_to_fail.is_empty() {
            parts.push(format!(
                "new failures: {}",
                join_preview(&flipped_to_fail, 3)
            ));
        }
        if !flipped_to_pass.is_empty() {
            parts.push(format!("new passes: {}", join_preview(&flipped_to_pass, 3)));
        }
        parts.join("; ")
    };
    let summary = format!(
        "What-if '{}={}' pass {}/{} -> {}/{}; {}",
        variable_name,
        override_display,
        baseline_pass,
        total_constraints,
        override_pass,
        total_constraints,
        details
    );
    show_command_result(ctx, summary.clone()).await;

    let mut payload = serde_json::json!({
        "variable": variable_name.clone(),
        "override_value": override_display,
        "baseline_count": baseline.len(),
        "override_count": overridden.len(),
        "baseline_pass": baseline_pass,
        "override_pass": override_pass,
        "flipped_count": flipped.len(),
        "flipped_to_fail": flipped_to_fail,
        "flipped_to_pass": flipped_to_pass,
        "flipped": flipped,
        "summary": summary,
    });

    let persist_summary = payload
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("what-if report").to_owned();
    let persist_uri = uri_str.clone();
    let persist_variable = variable_name.clone();
    let persist_payload = payload.clone();
    let report = tokio::task::spawn_blocking(move || {
        persist_whatif_report(
            "whatif",
            &persist_uri,
            &persist_variable,
            &persist_summary,
            &persist_payload,
        )
    })
    .await
    .unwrap_or_else(|e| {
        tracing::error!("whatif persist task panicked: {e}");
        serde_json::json!({"status": "error", "error": "task panicked"})
    });
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("report".to_owned(), report);
    }

    payload
}

pub(crate) async fn handle_whatif_sweep(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.len() < 7 {
        return error_json(
            "expected 7 arguments: uri, line, character, variable_name, start, end, steps",
        );
    }

    let uri_str = match require_string_arg(args, 0, "uri") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let line = match require_u32_arg(args, 1, "line") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let character = match require_u32_arg(args, 2, "character") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let variable_name = match require_string_arg(args, 3, "variable_name") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let start = match require_f64_arg(args, 4, "start") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let end = match require_f64_arg(args, 5, "end") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let steps = match require_usize_arg(args, 6, "steps") {
        Ok(value) => value,
        Err(error) => return error,
    };

    // Resolve element_id at cursor (LSP-side; service does the sweep).
    let (content, _graph, position_map) = match ctx.get_resolved(&uri_str).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            return error_json("document not in a workspace; call load_workspace first")
        }
        Err(_cancelled) => return error_json("request cancelled"),
    };

    let offset = position_to_offset(&Position { line, character }, &content);
    let Some(element_id) = position_map.element_id_at(offset) else {
        return error_json("no element at position");
    };

    let result_json = match ctx.service.whatif_sweep(
        &element_id,
        &variable_name,
        start,
        end,
        steps,
    ) {
        Ok(v) => v,
        Err(e) => return error_json(&format!("whatif.sweep failed: {e}")),
    };

    let raw_steps = result_json
        .get("steps")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let threshold_value = result_json
        .get("threshold")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let value_brief_json = |v: &serde_json::Value| -> Option<String> {
        if v.is_null() {
            None
        } else if let Some(i) = v.as_i64() {
            Some(i.to_string())
        } else if let Some(f) = v.as_f64() {
            Some(format!("{f}"))
        } else {
            None
        }
    };

    let steps_json: Vec<serde_json::Value> = raw_steps
        .iter()
        .map(|s| {
            let constraints = s
                .get("constraint_results")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            serde_json::json!({
                "value": s.get("value").cloned().unwrap_or(serde_json::Value::Null),
                "constraints": constraints,
            })
        })
        .collect();

    let total_constraints = raw_steps
        .first()
        .and_then(|s| s.get("constraint_results"))
        .and_then(|c| c.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);

    let step_pass = |s: &serde_json::Value| -> usize {
        s.get("constraint_results")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|item| {
                        item.get("satisfied")
                            .and_then(|b| b.as_bool())
                            .unwrap_or(false)
                    })
                    .count()
            })
            .unwrap_or(0)
    };
    let min_pass = raw_steps.iter().map(step_pass).min().unwrap_or(0);

    let first_fail = raw_steps.iter().find_map(|s| {
        let any_fail = s
            .get("constraint_results")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter().any(|item| {
                    !item
                        .get("satisfied")
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        if any_fail {
            value_brief_json(s.get("value").unwrap_or(&serde_json::Value::Null))
        } else {
            None
        }
    });

    let threshold_display =
        value_brief_json(&threshold_value).unwrap_or_else(|| "none".to_owned());

    let summary = format!(
        "What-if sweep '{}' points={} threshold={} first_fail={} min_pass={}/{}",
        variable_name,
        raw_steps.len(),
        threshold_display,
        first_fail.clone().unwrap_or_else(|| "none".to_owned()),
        min_pass,
        total_constraints
    );
    show_command_result(ctx, summary.clone()).await;
    let report_variable = variable_name.clone();

    let mut payload = serde_json::json!({
        "variable": report_variable.clone(),
        "steps": steps_json,
        "threshold": threshold_value,
        "points": raw_steps.len(),
        "constraints_per_step": total_constraints,
        "min_pass": min_pass,
        "first_fail": first_fail,
        "threshold_display": threshold_display,
        "summary": summary,
    });

    let persist_summary = payload
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("what-if sweep report").to_owned();
    let persist_uri = uri_str.clone();
    let persist_variable = report_variable.clone();
    let persist_payload = payload.clone();
    let report = tokio::task::spawn_blocking(move || {
        persist_whatif_report(
            "sweep",
            &persist_uri,
            &persist_variable,
            &persist_summary,
            &persist_payload,
        )
    })
    .await
    .unwrap_or_else(|e| {
        tracing::error!("whatif sweep persist task panicked: {e}");
        serde_json::json!({"status": "error", "error": "task panicked"})
    });
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("report".to_owned(), report);
    }

    payload
}

/// Diagram-oriented what-if: override a variable by name and return
/// overlay-compatible JSON (values, constraintResults, guardDiagnoses).
///
/// Args: `[uri, variable_name, override_value_string]`
///
/// Unlike `handle_whatif` this does **not** require a cursor position — the
/// variable is identified by name, making it suitable for diagram-panel
/// sliders / input fields.
pub(crate) async fn handle_diagram_whatif(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.len() < 3 {
        return error_json("expected 3 arguments: uri, variable_name, override_value");
    }
    let uri_str = match require_string_arg(args, 0, "uri") {
        Ok(v) => v,
        Err(e) => return e,
    };
    let variable_name = match require_string_arg(args, 1, "variable_name") {
        Ok(v) => v,
        Err(e) => return e,
    };
    let override_value = match require_string_arg(args, 2, "override_value") {
        Ok(v) => v,
        Err(e) => return e,
    };

    // Prefer the active orchestrator session for this URI; if absent, fall
    // back to a whole-graph baseline so the diagram overlay panel still
    // works before any orchestrate.start has been called.
    let session_key = format!("orch:{}", uri_str);
    match ctx.service.whatif(
        &variable_name,
        &override_value,
        Some(&session_key),
    ) {
        Ok(v) => v,
        Err(e) => {
            let session_missing = matches!(
                &e,
                sysml_service::ServiceError::InvalidInput(m)
                    if m.contains("session_key") && m.contains("not found")
            );
            if session_missing {
                match ctx.service.whatif(
                    &variable_name,
                    &override_value,
                    None,
                ) {
                    Ok(v) => v,
                    Err(e2) => error_json(&format!("whatif failed: {e2}")),
                }
            } else {
                error_json(&format!("whatif failed: {e}"))
            }
        }
    }
}

// ============================================================================
// Diagram commands
// ============================================================================

pub(crate) async fn handle_diagram_open(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    tracing::info!(
        args_len = args.len(),
        args = %serde_json::to_string(args).unwrap_or_default(),
        "handle_diagram_open called"
    );
    if args.is_empty() {
        return error_json("expected arguments: uri [, view_type]");
    }

    let uri_str = match require_string_arg(args, 0, "uri") {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(arg0 = ?args.first(), "uri arg is not a string");
            return error;
        }
    };
    let view_type_value = match optional_string_arg(args, 1, "view_type") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let view_type_str = view_type_value.as_deref().unwrap_or("general");
    let view_type = diagram::parse_view_type(view_type_str);

    // The service owns ViewModel projection and open-diagram state.
    let model = match ctx.service.diagram_open(&uri_str, Some(view_type_str)) {
        Ok(v) => v,
        Err(e) => return error_json(&e.to_string()),
    };

    let params = diagram::DiagramSetViewModelParams {
        uri: uri_str.clone(),
        view_type: diagram::view_type_name(view_type).to_owned(),
        view_model: model,
    };
    ctx.client
        .send_notification::<DiagramSetViewModelNotification>(params)
        .await;


    show_command_result(
        ctx,
        format!("Diagram generated for '{}' ({})", uri_str, view_type_str),
    )
    .await;

    serde_json::json!({
        "uri": uri_str,
        "viewType": view_type_str,
        "status": "sent",
    })
}

#[tracing::instrument(level = "debug", skip(ctx, args))]
pub(crate) async fn handle_diagram_view(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.len() < 2 {
        return error_json("expected 2 arguments: uri, view_type");
    }

    let uri_str = match require_string_arg(args, 0, "uri") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let view_type_value = match require_string_arg(args, 1, "view_type") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let view_type_str = view_type_value.as_str();
    let view_type = diagram::parse_view_type(view_type_str);

    // The service owns ViewModel projection and open-diagram state.
    let model = match ctx.service.diagram_view(&uri_str, view_type_str) {
        Ok(v) => v,
        Err(e) => return error_json(&e.to_string()),
    };

    let params = diagram::DiagramSetViewModelParams {
        uri: uri_str.clone(),
        view_type: diagram::view_type_name(view_type).to_owned(),
        view_model: model,
    };
    ctx.client
        .send_notification::<DiagramSetViewModelNotification>(params)
        .await;


    show_command_result(
        ctx,
        format!(
            "Diagram view switched to '{}' for '{}'",
            view_type_str, uri_str
        ),
    )
    .await;

    serde_json::json!({
        "uri": uri_str,
        "viewType": view_type_str,
        "status": "sent",
    })
}

#[tracing::instrument(level = "debug", skip(ctx, args))]
pub(crate) async fn handle_diagram_export(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.is_empty() {
        return error_json("expected arguments: uri [, view_type]");
    }

    let uri_str = match require_string_arg(args, 0, "uri") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let view_type_value = match optional_string_arg(args, 1, "view_type") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let view_type_str = view_type_value.as_deref().unwrap_or("general");

    // The service owns ViewModel projection. This export shares the ad-hoc
    // view path and its expansion-state contract.
    let model = match ctx.service.diagram_view(&uri_str, view_type_str) {
        Ok(v) => v,
        Err(e) => return error_json(&e.to_string()),
    };

    show_command_result(
        ctx,
        format!("Diagram exported for '{}' ({})", uri_str, view_type_str),
    )
    .await;

    serde_json::json!({
        "uri": uri_str,
        "viewType": view_type_str,
        "viewModel": model,
    })
}

/// Handle expand/collapse of a diagram node.
///
/// Toggles the expanded state of a typed state usage node, re-generates
/// the diagram, and sends the updated ViewModel to the client.
///
/// Arguments: [uri, elementId, expanded (bool)]
#[tracing::instrument(level = "info", skip_all)]
pub(crate) async fn handle_diagram_expand(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.len() < 3 {
        return error_json("expected 3 arguments: uri, elementId, expanded");
    }

    let uri_str = match require_string_arg(args, 0, "uri") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let element_id = match require_string_arg(args, 1, "elementId") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let expanded = args.get(2).and_then(|v| v.as_bool()).unwrap_or(true);

    // Resolve the new full expanded-node set from the existing per-URI set
    // toggled by (element_id, expanded). The service command replaces state
    // with this set + re-projects, so we hand it the authoritative view.
    let mut expanded_set: std::collections::HashSet<String> = ctx
        .service
        .diagram_manager()
        .expanded_nodes
        .get(&uri_str)
        .map(|e| e.value().clone())
        .unwrap_or_default();
    if expanded {
        expanded_set.insert(element_id.clone());
    } else {
        expanded_set.remove(&element_id);
    }

    // View-type is whatever the URI is currently displaying; falls back to
    // StateTransition for the toggle-without-prior-open edge case, matching
    // the prior LSP default.
    let view_type = ctx
        .service
        .diagram_manager()
        .open_diagrams
        .get(&uri_str)
        .map(|v| *v.value())
        .unwrap_or(sysml_diagram::ViewType::StateTransition);
    let view_type_str = diagram::view_type_name(view_type);

    let expanded_vec: Vec<String> = expanded_set.into_iter().collect();
    let model = match ctx
        .service
        .diagram_expand(&uri_str, view_type_str, &expanded_vec)
    {
        Ok(v) => v,
        Err(e) => return error_json(&e.to_string()),
    };

    let params = diagram::DiagramSetViewModelParams {
        uri: uri_str.clone(),
        view_type: view_type_str.to_owned(),
        view_model: model,
    };
    ctx.client
        .send_notification::<DiagramSetViewModelNotification>(params)
        .await;


    tracing::info!(
        uri = %uri_str,
        element_id = %element_id,
        expanded,
        "diagram node expand/collapse toggled"
    );

    serde_json::json!({
        "uri": uri_str,
        "elementId": element_id,
        "expanded": expanded,
        "status": "sent",
    })
}

/// Build a model tree for the Model Explorer sidebar.
///
/// Returns a JSON array of tree nodes with id, name, kind, uri, range, children.
///
/// S2.T6 (LSP-69, 2026-05-08): tree compute delegates to
/// Bucket B / B4 P1: the multi-URI driver + span→Range conversion live on
/// the service (`sysml.workspace.model_tree`). The LSP handler is a thin
/// marshal that flattens the per-URI grouped service shape to the
/// existing flat-array wire shape the editor consumes.
pub(crate) async fn handle_model_tree(
    ctx: &CommandContext<'_>,
    _args: &[serde_json::Value],
) -> serde_json::Value {
    match ctx.service.workspace_model_tree(None, Some("full")) {
        Ok(groups) => {
            let nodes: Vec<serde_json::Value> = groups
                .into_iter()
                .flat_map(|group| {
                    group
                        .nodes
                        .into_iter()
                        .map(|n| serde_json::to_value(n).unwrap_or(serde_json::Value::Null))
                })
                .collect();
            serde_json::Value::Array(nodes)
        }
        Err(e) => error_json(&e.to_string()),
    }
}

// ── Diagram edit command ─────────────────────────────────────────────

/// Handle diagram edit requests (create/delete/rename) from the webview.
///
/// The first argument is a JSON object matching the service-side
/// `DiagramEditRequest`. The service computes the resulting workspace
/// edit; this LSP shim applies it via `workspace/applyEdit` and reshapes
/// the result into the JSON-RPC response.
#[tracing::instrument(level = "debug", skip(ctx, args))]
pub(crate) async fn handle_diagram_edit(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    let Some(arg) = args.first() else {
        return error_json("missing diagram edit request argument");
    };

    let computed = match ctx.service.diagram_edit(arg) {
        Ok(c) => c,
        Err(e) => return error_json(&format!("diagram edit failed: {e}")),
    };

    let workspace_edit = service_edit_to_lsp(&computed.workspace_edit);
    if computed.workspace_edit.changes.is_empty() {
        return error_json(&format!(
            "no edit produced for action '{}'",
            computed.action
        ));
    }

    match ctx.client.apply_edit(workspace_edit).await {
        Ok(resp) if resp.applied => {
            show_command_result(ctx, computed.status_message.clone()).await;
            let mut result = serde_json::json!({ "status": "applied" });
            if let serde_json::Value::Object(extra) = &computed.status_payload {
                let obj = result.as_object_mut().unwrap();
                for (k, v) in extra {
                    obj.insert(k.clone(), v.clone());
                }
            }
            // Action-specific shape: `create` exposes `elementType`; others use
            // the plain `action` string in addition to the payload.
            if computed.action != "create" {
                result["action"] = serde_json::json!(computed.action);
            }
            result
        }
        Ok(resp) => {
            let reason = resp.failure_reason.unwrap_or_else(|| "unknown".to_owned());
            error_json(&format!("edit not applied: {reason}"))
        }
        Err(e) => error_json(&format!("apply_edit failed: {e}")),
    }
}

fn service_edit_to_lsp(
    edit: &sysml_service::diagram_edit::DiagramWorkspaceEdit,
) -> tower_lsp::lsp_types::WorkspaceEdit {
    let mut changes: std::collections::HashMap<
        tower_lsp::lsp_types::Url,
        Vec<tower_lsp::lsp_types::TextEdit>,
    > = std::collections::HashMap::new();
    for file_edits in &edit.changes {
        let url = tower_lsp::lsp_types::Url::parse(&file_edits.uri)
            .or_else(|_| tower_lsp::lsp_types::Url::from_file_path(&file_edits.uri))
            .unwrap_or_else(|_| {
                tower_lsp::lsp_types::Url::parse("file:///unknown").unwrap()
            });
        let edits: Vec<tower_lsp::lsp_types::TextEdit> = file_edits
            .edits
            .iter()
            .map(|e| tower_lsp::lsp_types::TextEdit {
                range: tower_lsp::lsp_types::Range {
                    start: tower_lsp::lsp_types::Position {
                        line: e.line_start,
                        character: e.col_start,
                    },
                    end: tower_lsp::lsp_types::Position {
                        line: e.line_end,
                        character: e.col_end,
                    },
                },
                new_text: e.new_text.clone(),
            })
            .collect();
        changes.insert(url, edits);
    }
    tower_lsp::lsp_types::WorkspaceEdit {
        changes: Some(changes),
        ..Default::default()
    }
}

// ============================================================================
// Orchestrator commands
// ============================================================================

pub(crate) async fn handle_orchestrate_start(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.is_empty() {
        return error_json("expected argument: uri");
    }

    let uri_str = match require_string_arg(args, 0, "uri") {
        Ok(value) => value,
        Err(error) => return error,
    };

    // Service does the canonical compile: Q-LSP-3 Pest re-parse from
    // disk for AssignmentActionUsage support, elaborate,
    // build_workspace_orchestrator (all subsystem types — SMs + ODE +
    // physics + computed expressions), seed context, first step,
    // insert session.
    let (session_id, snapshot) = match ctx.service.orchestrate_start(&uri_str) {
        Ok(v) => v,
        Err(e) => return error_json(&e.to_string()),
    };

    let subsystem_info: Vec<serde_json::Value> = snapshot
        .subsystem_states
        .iter()
        .map(|(name, state)| {
            serde_json::json!({
                "name": name,
                "kind": "stateMachine",
                "state": state.current_state,
                "completed": state.completed,
                "outputs": state.outputs,
                "sends": [],
            })
        })
        .collect();

    if subsystem_info.is_empty() {
        return error_json("no state machines found in document");
    }

    show_command_result(
        ctx,
        format!(
            "Orchestrator started with {} subsystem(s), {} context vars",
            subsystem_info.len(),
            snapshot.variables.len(),
        ),
    )
    .await;

    serde_json::json!({
        "session_id": session_id,
        "subsystems": subsystem_info,
        "tick": snapshot.tick,
        "time_ms": snapshot.time_ms,
    })
}

/// Legacy orchestrator.step — thin wrapper around unified sessions_step.
pub(crate) async fn handle_orchestrate_step(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    handle_sessions_step(ctx, args).await
}

/// Legacy orchestrator.inject — thin wrapper around unified sessions_inject.
pub(crate) async fn handle_orchestrate_inject(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    handle_sessions_inject(ctx, args).await
}

pub(crate) async fn handle_orchestrate_stop(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.is_empty() {
        return error_json("expected argument: session_id");
    }

    let session_id = match require_string_arg(args, 0, "session_id") {
        Ok(value) => value,
        Err(error) => return error,
    };

    match ctx.service.sessions_stop(&session_id) {
        Ok(()) => {
            show_command_result(ctx, format!("Orchestrator session '{}' stopped", session_id))
                .await;
            serde_json::json!({"status": "stopped"})
        }
        Err(_) => error_json("session not found"),
    }
}

// ============================================================================
// Scenario commands (Phase 9.2)
// ============================================================================

pub(crate) async fn handle_scenario_run(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.len() < 2 {
        return error_json("expected 2 arguments: uri, case_name");
    }

    // Positional `uri` stays on the wire for client arity compat
    // (validated, unused): the service call is workspace-scoped since the
    // workspace-scope collapse (2026-07-16).
    let _uri_str = match require_string_arg(args, 0, "uri") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let case_name = match require_string_arg(args, 1, "case_name") {
        Ok(value) => value,
        Err(error) => return error,
    };

    // Delegate the full orchestrator composition to the service. See
    // sysml-service::scenario::run_scenario for the canonical
    // implementation; this handler is now a marshal + UI nudge.
    let payload = match ctx.service.scenario_run(&case_name, None) {
        Ok(v) => v,
        Err(e) => return error_json(&e.to_string()),
    };

    let verdict_str = payload
        .get("verdict")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_owned();
    let tick_count = payload
        .get("trace")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    show_command_result(
        ctx,
        format!(
            "Scenario '{}': {} ({} ticks)",
            case_name, verdict_str, tick_count,
        ),
    )
    .await;

    payload
}

// ============================================================================
// Timeline commands (Phase 9.2)
// ============================================================================

pub(crate) async fn handle_timeline_get_trace(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    let session_id = match require_string_arg(args, 0, "session_id") {
        Ok(value) => value,
        Err(error) => return error,
    };
    match ctx.service.timeline_get_trace(&session_id) {
        Ok(v) => v,
        Err(e) => error_json(&e.to_string()),
    }
}

pub(crate) async fn handle_timeline_get_snapshot(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    let session_id = match require_string_arg(args, 0, "session_id") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let tick = args.get(1).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    match ctx.service.timeline_get_snapshot(&session_id, tick) {
        Ok(v) => v,
        Err(e) => error_json(&e.to_string()),
    }
}

// ── Monte Carlo ──

pub(crate) async fn handle_montecarlo_run(
    ctx: &CommandContext<'_>,
    args: &[serde_json::Value],
) -> serde_json::Value {
    if args.is_empty() {
        return error_json("expected arguments: uri, config");
    }

    // Positional `uri` stays on the wire for client arity compat
    // (validated, unused): the service call is workspace-scoped since the
    // workspace-scope collapse (2026-07-16).
    let _uri_str = match require_string_arg(args, 0, "uri") {
        Ok(value) => value,
        Err(error) => return error,
    };

    let config_json = args.get(1).cloned().unwrap_or(serde_json::json!({}));

    match ctx.service.montecarlo(&config_json) {
        Ok(value) => value,
        Err(e) => error_json(&e.to_string()),
    }
}
