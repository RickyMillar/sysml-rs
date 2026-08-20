# sysml-api

The REST / WebSocket / SSE transport over `sysml-service` — exposes the shared SysML v2 command hub as an axum HTTP server.

`Layer 5 · tooling` · `transport: REST + WS + SSE` · `crate-type: lib + bin` · `axum · tokio`

## Overview

`sysml-api` is one of four transports that front the same `sysml-service` instance (the others are the CLI, the LSP server, and the MCP server). It owns *no* domain state of its own: `AppState` is a thin wrapper around an `Arc<SysmlService>` plus a background session-reaper guard. Every request is translated into a service call, and the service's salsa-backed `AnalysisHost` + store do the real work.

The server speaks three protocols on one router:

**REST.**

42 explicit JSON routes plus 124 auto-generated command routes. Reads are unauthenticated; mutations are gated by a bearer token when configured.

**WebSocket.**

`GET /lsp` bridges a full LSP session over WS; `GET /api/sessions/:id/events` streams live simulation/action session events.

**SSE.**

`GET /v1/progress` streams `ProgressEvent`s (library load, workspace index, dependency fetch, ready).

The binary (`src/main.rs`) runs the HTTP API on `0.0.0.0:8080` by default; passing `--mcp` additionally spawns an MCP stdio handler that *shares the same service instance*, so files loaded over HTTP are visible to an AI agent and vice versa.

## Where it sits

```text
clients HTTP / fetch WebSocket (LSP) SSE (progress) MCP stdio (--mcp)
▼
L5 transport sysml-api router: read · write · inventory middleware: auth · CORS · 50 MB limit
▼
L5 hub sysml-service · Arc<SysmlService> sysml-lsp-server sysml-mcp
▼
below sysml-store sysml-runtime sysml-core sysml-id
```

All four transports drive a single shared service hub; `sysml-api` contributes only the HTTP/WS/SSE surface and request-to-command translation.

## Public API

The library surface is small — the value is in the router and request/response types.

#### `— *struct AppState` — *state*

Thin wrapper around the shared service. Built two ways:

- `AppState::new()` — empty service over a fresh `InMemoryStore` behind a `RwLock`.

- `AppState::with_service(Arc<SysmlService>)` — wrap an existing service (used by the `--mcp` path so HTTP and MCP share state).

Holds an opaque `SessionReaperGuard`. The reaper is spawned at construction *only when a tokio runtime is active* (no-op in sync tests) and is aborted on `Drop`. `AppState::with_store(..)` no longer exists — it predates the service hub.

#### `— *fn create_router(state: Arc<AppState>) -> Router` — *router*

Assembles three route groups — `read_routes` (no auth), `write_routes` (auth layer), and `inventory_routes()` (auth layer) — then applies a global 50 MB body limit and a wide-open CORS layer. Returns the full `axum::Router`.

#### `— *async fn run_server(addr: &str) -> Result<(), Box<dyn Error>>` — *entry*

Constructs `AppState::new()`, builds the router, binds a `TcpListener`, and serves it with `axum::serve`. The `--mcp` binary path builds the router manually instead so it can clone the service into the MCP handler.

#### `— *fn inventory_routes() -> Router<Arc<AppState>>` — *router*

Iterates `sysml_service::registered_commands()` and, for each, registers `POST /api/commands/{name}` that dispatches the JSON body through `sysml_service::execute_command`. Named, discoverable aliases for the generic `POST /api/command` dispatcher. At authoring time there are **124** registered commands.

## REST routes

42 explicit routes. Reads need no auth; writes are gated by `require_auth` when `SYSML_API_TOKEN` is set. Filter below to find a route.

| Method | Path | Handler | Auth |
|---|---|---|---|
| GET | /health | health | — |
| GET | /projects | list_projects | — |
| GET | /projects/:project_id/commits | list_commits | — |
| GET | /projects/:project_id/commits/:commit_id/model | get_model | — |
| GET | /models | list_models | — |
| GET | /models/:uri/find | find_elements | — |
| GET | /models/:uri/stats | model_stats | — |
| GET | /models/:uri/tree | model_tree | — |
| GET | /models/:uri/unverified | unverified | — |
| GET | /models/:uri/elements/:id | get_element | — |
| GET | /models/:uri/elements/:id/children | get_children | — |
| GET | /models/:uri/elements/:id/ancestors | get_ancestors | — |
| GET | /models/:uri/elements/:id/descendants | get_descendants | — |
| GET | /models/:uri/trace | trace_matrix | — |
| GET | /models/:uri/diagnostics | get_diagnostics | — |
| GET | /models/:uri/export/json | export_json | — |
| GET | /models/:uri/views | views_list | — |
| GET | /models/:uri/views/:view_id/render | views_render | — |
| GET | /models/:uri/views/by_viewpoint/:viewpoint_id | views_by_viewpoint | — |
| GET | /models/:uri/viewpoints/by_stakeholder/:stakeholder_id | viewpoints_by_stakeholder | — |
| GET | /workspace/files | workspace_files | — |
| GET | /commands | commands | — |
| GET | /v1/progress | progress_sse::progress_sse_handler (SSE) | — |
| GET | /v1/readiness/:uri | readiness_for | — |
| GET | /lsp | lsp_ws::lsp_ws_handler (WS) | — |
| GET | /api/sessions/:id/events | session_ws::session_ws_handler (WS) | — |
| POST | /projects/:project_id/commits/:commit_id/model | store_model | token |
| POST | /sources | load_source | token |
| POST | /api/query | query_engine | token |
| POST | /models/:uri/check | check_constraints | token |
| POST | /views/scratch | views_create_scratch | token |
| POST | /eval | eval_expression | token |
| POST | /files | load_file | token |
| POST | /sessions/simulate/start | simulate_start | token |
| POST | /sessions/:key/step | simulate_step | token |
| DELETE | /sessions/:key | simulate_stop | token |
| POST | /sessions/action/start | action_start | token |
| POST | /sessions/action/:key/step | action_step | token |
| POST | /sessions/continuous/start | continuous_start | token |
| POST | /sessions/orchestrator/:key/step | orchestrator_step | token |
| DELETE | /sessions/orchestrator/:key | orchestrator_stop | token |
| POST | /api/command | dispatch_command | token |

>  **Plus 124 auto-generated routes.** `inventory_routes()` registers `POST /api/commands/{name}` for every command from `registered_commands()` (e.g. `sysml.load_workspace`, `sysml.load_file`, `sysml.query`, …). They are token-gated and are named aliases for `POST /api/command`. `GET /commands` returns the live catalog so the list never drifts.

## Readiness envelope & progress

URI-keyed read endpoints (`find`, `stats`, `tree`, `unverified`, `diagnostics`, and the `elements/:id*` navigation routes) accept an optional `?with_readiness=1` flag (any truthy value that is not `0`/`false`/empty).

**Without the flag (legacy shape).**

```
GET /models/coffee.sysml/diagnostics

[
  { "severity": "error", "message": "…" }
]
```

**With `?with_readiness=1`.**

```
GET /models/coffee.sysml/diagnostics?with_readiness=1

{
  "data":      [ /* same array */ ],
  "readiness": { /* Readiness snapshot */ }
}
```

`GET /v1/readiness/:uri` returns the `Readiness` snapshot directly, and `GET /v1/progress` streams `ProgressEvent`s over SSE — each event's SSE `event` name is the variant discriminant (`library_load`, `workspace_index`, `dependency_fetch`, `refresh`, `ready`); lagged subscribers get `event: lagged` with the dropped count and a keep-alive comment every 15 s.

## Source modules

| File | Responsibility | Key items |
|---|---|---|
| src/lib.rs | Request/response types, all REST handlers, error mapping, router assembly, auth/CORS/body-limit middleware, test module | AppState · create_router · inventory_routes · require_auth · service_err_response |
| src/main.rs | Binary entry point; default HTTP server, optional shared MCP stdio handler via `--mcp` | main · AppState::new · sysml_mcp::serve |
| src/lsp_ws.rs | WebSocket transport bridging an LSP session over WS at `/lsp` | lsp_ws_handler |
| src/progress_sse.rs | SSE stream over `SysmlService::subscribe_progress` | progress_sse_handler |

## Usage

Embed the router in a test or a custom binary. This compiles against the current API (no `with_store`).

```
use std::sync::Arc;
use sysml_api::{AppState, create_router};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Empty in-memory service. The session reaper spawns on the tokio runtime
    // and is aborted when `state` is dropped.
    let state = Arc::new(AppState::new());
    let app = create_router(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

Or just run the shipped binary:

```
# REST API on 0.0.0.0:8080
cargo run -p sysml-api

# REST + shared MCP stdio (same service instance)
cargo run -p sysml-api -- --mcp

# Gate all mutations behind a bearer token
SYSML_API_TOKEN=secret cargo run -p sysml-api
```

Tests drive the router in-process with `tower::ServiceExt::oneshot` — no port binding:

```
use tower::ServiceExt;
use axum::{body::Body, http::Request};

let app = create_router(Arc::new(AppState::new()));
let resp = app
    .oneshot(Request::builder().uri("/health").body(Body::empty())?)
    .await?;
assert_eq!(resp.status(), 200);
```

## Dependencies

**Upstream (this crate depends on).**

- `sysml-service` — the command hub: `SysmlService`, `registered_commands`, `execute_command`, `session_reaper`, `readiness`

- `sysml-lsp-server` — LSP backend bridged over the `/lsp` WebSocket

- `sysml-mcp` — MCP server reused by the `--mcp` binary path

- `sysml-store` — `InMemoryStore`, `SnapshotMeta` for default construction

- `sysml-runtime` (serde) — execution/IR/physics types in session payloads

- `sysml-core` (serde) · `sysml-id` — model types and ids

- `axum` (ws) · `tokio` · `tower-http` · `tower-lsp` · `serde` · `serde_json` · `ciborium`

**Downstream (depends on this crate).**

None inside the workspace — `sysml-api` is a terminal transport. It is consumed as a running server / binary by external HTTP, WebSocket, and SSE clients, and by the standalone simulation app.

## Pitfalls & invariants

- **No `AppState::with_store`.** Construct with `AppState::new()` or `AppState::with_service(..)`. The store lives inside the service now.

- **Reaper needs a runtime.** `AppState::new()` only spawns the session reaper inside a tokio runtime; in a sync test it is a no-op. Dropping the state aborts the task.

- **Auth is off by default.** With `SYSML_API_TOKEN` unset, all mutations are open (back-compat). When set, write/inventory routes require `Authorization: Bearer <token>`.

- **CORS is wide open.** `allow_origin/methods/headers(Any)` applied globally — intended for local development, not a hardened deployment.

- **Error mapping.** `service_err_response` maps `ServiceError::{ElementNotFound,NotFound}` → 404, `InvalidInput` → 400, `Store("no store configured")` → 503, everything else → 500, with body `{ "error": String }`.

- **Prefer dispatch over new aliases.** New structured reads should go through `POST /api/query` or an inventory command, not a hand-coded REST alias.

Part of the [sysml-rs](../../../README.md) workspace · regenerated 2026-06-03
