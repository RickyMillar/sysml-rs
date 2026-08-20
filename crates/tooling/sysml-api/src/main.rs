//! SysML API server with optional MCP (Model Context Protocol) support.
//!
//! By default, runs the HTTP REST API on port 8080.
//! With `--mcp`, also spawns an MCP stdio handler sharing the same service
//! instance — so files loaded via the API are visible to Claude, and vice versa.
//!
//! The workflow sidecar (attestations, comments, approvals) is DURABLE by
//! default: an append-only JSONL in the local data directory (steward
//! ruling 2026-07-16 — an in-memory default would silently discard the
//! audit trail on restart). Override with `--workflow-store=<path>`.

use std::sync::Arc;

use sysml_service::SysmlService;
use sysml_store::{InMemoryStore, JsonlWorkflowStore};

/// `$XDG_DATA_HOME` (or `~/.local/share`) `/sysml-rs/workflow.jsonl`.
fn default_workflow_store_path() -> std::path::PathBuf {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    data_home.join("sysml-rs").join("workflow.jsonl")
}

#[allow(clippy::unwrap_used)] // Top-level entry point — panic on server failure is intentional
#[allow(clippy::print_stderr)] // CLI startup banner
#[allow(clippy::exit)] // fail-hard on an unusable audit trail is deliberate
fn build_service(args: &[String]) -> Arc<SysmlService> {
    let workflow_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--workflow-store="))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(default_workflow_store_path);

    // Fail hard if the log cannot be opened/replayed: a corrupt audit
    // trail must stop the server, never silently degrade to in-memory.
    let (workflow_store, recovery) = match JsonlWorkflowStore::open(&workflow_path) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!(
                "FATAL: workflow sidecar log unusable at {}: {e}",
                workflow_path.display()
            );
            std::process::exit(1);
        }
    };
    if let Some(tail) = recovery.torn_tail_discarded {
        eprintln!(
            "WARNING: discarded a torn partial append from {}: {tail:?}",
            workflow_path.display()
        );
    }
    eprintln!("  workflow sidecar: {}", workflow_path.display());

    let store = Arc::new(std::sync::RwLock::new(InMemoryStore::new()));
    Arc::new(SysmlService::with_store(store).with_workflow_store(Arc::new(workflow_store)))
}

#[allow(clippy::unwrap_used)] // Top-level entry point — panic on server failure is intentional
#[allow(clippy::print_stderr)] // CLI startup banner
#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mcp_mode = args.iter().any(|a| a == "--mcp");
    let addr = args
        .iter()
        .find(|a| !a.starts_with('-') && *a != &args[0])
        .cloned()
        .unwrap_or_else(|| "0.0.0.0:8080".to_owned());

    if mcp_mode {
        // Shared service: both HTTP API and MCP stdio use the same instance.
        // Files loaded via the sim app (HTTP) are visible to Claude (MCP), and vice versa.
        eprintln!("SysML API+MCP server listening on {addr}");
        eprintln!("  HTTP API: http://{addr}");
        eprintln!("  MCP: stdio (JSON-RPC)");

        let service = build_service(&args);
        let state = Arc::new(sysml_api::AppState::with_service(service.clone()));
        let app = sysml_api::create_router(state);

        // Spawn the HTTP server in a background task
        let http_handle = tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
            axum::serve(listener, app).await.unwrap();
        });

        // Run MCP on stdio in the foreground (blocks until Claude disconnects)
        if let Err(e) = sysml_mcp::serve(service).await {
            eprintln!("MCP error: {e}");
        }

        // MCP finished — shut down HTTP too
        http_handle.abort();
    } else {
        eprintln!("SysML API server listening on {addr}");
        let state = Arc::new(sysml_api::AppState::with_service(build_service(&args)));
        let app = sysml_api::create_router(state);
        let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
        axum::serve(listener, app).await.unwrap();
    }
}
