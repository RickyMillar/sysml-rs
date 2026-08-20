# sysml-mcp

MCP (Model Context Protocol) server that exposes the unified `SysmlService` to AI agents as **125** stdio JSON-RPC tools. A pure transport adapter — every tool delegates to the service layer.

`Layer 5 · tooling` · `MCP server` · `crate-type: bin` · `transport: stdio JSON-RPC` · `framework: rmcp`

## Overview

`sysml-mcp` is one of four transports over the [sysml-service](../sysml-service/README.md) command hub (the others are CLI, LSP, and REST). It speaks the [Model Context Protocol](https://modelcontextprotocol.io) so an AI agent — Claude Desktop, Claude Code, or any MCP client — can load SysML v2 models, query the model graph, run simulations and analyses, and drive interactive sessions, all by calling named tools.

The crate enforces a single architectural invariant: **no domain logic lives here.** Every `#[tool]` handler is a thin shim that converts MCP request parameters into JSON and calls `sysml_service::execute_command(service, "sysml.<dotted>", params)`. The service registry (inventory-based) is the source of truth; this crate only adapts it to the wire.

>  **The live tool list is the catalog, not this page.** Call the `sysml_command_catalog` tool (or `SysmlService::command_catalog()`) to enumerate every operation with its parameters at runtime. The category groupings below are hand-authored for orientation and *can drift* — when in doubt, trust the catalog. A unit test (`all_commands_have_mcp_tools`) guarantees every service command has a matching tool.

## Where it sits

```text
client Claude Desktop / Claude Code / any MCP client
▼ stdio · JSON-RPC · `notifications/message` progress stream
this crate sysml-mcp · serve() SysmlMcpHandler ToolRouter (125 #[tool])
▼ dispatch_to_service(service, "sysml.<cmd>", json)
hub sysml-service · execute_command (inventory dispatch)
▼
domain SysmlService · salsa AnalysisHost · sessions · runtime
```

## Dispatch flow

```
stdio (JSON-RPC over stdin/stdout; logs → stderr)
  → rmcp transport (rmcp::transport::io::stdio)
    → SysmlMcpHandler (ServerHandler; holds Arc<SysmlService> + ToolRouter<Self>)
      → #[tool] method matched by name
        → dispatch_to_service(&self.service, "sysml.<command>", json_params)
          → sysml_service::execute_command(...)   // inventory lookup
            → SysmlService method
```

Two helpers back every handler:

- `dispatch_to_service(service, command, params)` — the common path. Maps a service `Ok` to a JSON `CallToolResult` and a service `Err` to `CallToolResult::error(...)` (never a protocol-level failure).

- `dispatch_to_service_with_readiness(service, command, params, uri)` — additive variant that appends a `_readiness` field (library/project/file load state) to the response envelope so an agent can decide whether to retry while indexing is still in flight. Currently wired on the tools whose callers most often race indexing: `sysml_load_file`, `sysml_load_workspace`, `sysml_diagnostics`, `sysml_hover`, `sysml_completion`.

## Wiring it into an MCP client

The server is a long-lived subprocess that the client launches and talks to over stdio. Build the binary, then register it.

```
# build
cargo build -p sysml-mcp --release
# binary at target/release/sysml-mcp
```

Claude Desktop (`claude_desktop_config.json`) or a project `.mcp.json`:

```
{
  "mcpServers": {
    "sysml": {
      "command": "/abs/path/to/target/release/sysml-mcp",
      "args": [],
      "env": { "RUST_LOG": "sysml_mcp=info" }
    }
  }
}
```

> ⚠  **stdout is the transport.** Any `println!` or stray stdout write corrupts the JSON-RPC stream. The binary routes *all* `tracing` output to **stderr**; keep it that way.

## Tool families

All **125** tools (verified `rg -c '#[tool' src/lib.rs`). Tool names use underscores; the backing service command uses dots — the mapping is mechanical (`sysml.sessions.fork` → `sysml_sessions_fork`). This grouping is for orientation only; the canonical list is `sysml_command_catalog`.

filter

| Family | Count | Representative tools |
|---|---|---|
| Loading | 7 | load_source, load_file, load_file_ts, load_workspace, unload_file, get_source, loaded_uris |
| Model query | 9 | query, find, element, children, ancestors, descendants, stats, model_tree, trace |
| Diagnostics & analysis | 9 | diagnostics, constraint_check, expression_eval, expression_ast, unverified, trace_matrix, flow_inspect, causation_trace, verify |
| Evaluate (cases) | 6 | evaluate, evaluate_constraints, evaluate_verification_cases, evaluate_analysis_cases, evaluate_calculations, evaluate_expression |
| Solve / analysis cases | 4 | solve, analysis_run, montecarlo_run, sensitivity_analyze |
| Trade studies & what-if | 5 | whatif, whatif_sweep, trade_study, trade_study_ode_sweep, aggregate |
| Verification w/ simulation | 4 | verify_with_simulation, verify_with_simulation_trace, verify_timeline, workspace_verify |
| Simulation (discrete) | 5 | simulate_start, simulate_step, simulate_stop, simulate_continuous_start, simulate_continuous_auto |
| Action execution | 3 | action_start, action_step, action_run |
| Orchestration | 5 | orchestrate_start, orchestrate_step, orchestrate_inject, orchestrate_stop, orchestrate_workspace_start |
| Debug (breakpoints) | 3 | breakpoint_set, breakpoint_clear, breakpoint_list |
| Sessions (lifecycle) | 11 | sessions_list, sessions_info, sessions_stop, sessions_reap, sessions_reset, sessions_rename, sessions_quota, sessions_step, sessions_inject, sessions_fork, sessions_subsystems |
| Sessions (overrides) | 3 | sessions_step_with_overrides, sessions_inject_with_overrides, sessions_fork_with_overrides |
| Sessions (timeseries) | 3 | sessions_timeseries, sessions_timeseries_decimated, sessions_timeseries_names |
| Sessions (diff & topology) | 3 | sessions_diff, sessions_diff_timeline, sessions_topology |
| Session archive | 4 | sessions_archive_list, sessions_archive_get, sessions_archive_mark_golden, sessions_archive_unmark_golden |
| Batch (R5.0) | 4 | batch_create, batch_status, batch_results, batch_slice |
| IDE / LSP-style | 11 | outline, references, goto_definition, hover, completion, completion_resolve, rename, format_document, code_action_list, inspect, parse |
| Diagram & export | 3 | diagram_edit, export_json, export_plantuml |
| Views & viewpoints | 5 | views_list, views_render, views_by_viewpoint, views_create_scratch, viewpoints_by_stakeholder |
| Store / persistence | 5 | store_save, store_load, store_latest, store_projects, store_history |
| Workspace & capabilities | 6 | workspace_info, workspace_files, workspace_refresh, workspace_capabilities, system_capabilities, readiness |
| Cache & salsa stats | 5 | cache_clear, cache_status, cache_rebuild, salsa_stats, salsa_stats_reset |
| Dependencies | 1 | dependency_status |
| Meta | 1 | command_catalog (calls SysmlService::command_catalog() directly) |

Counts sum to 125. The grouping is editorial; only `sysml_command_catalog` is authoritative.

## Key types & entry points

#### `pub async fn serve(service: Arc<SysmlService>) -> Result<(), Box<dyn Error>>` — *fn*

Public entry point. Builds the stdio transport (`rmcp::transport::io::stdio()`), constructs `SysmlMcpHandler`, serves it, and blocks on `server.waiting()` until the client disconnects. Called by `main.rs` after constructing an empty service and spawning the shared session reaper.

#### `pub struct SysmlMcpHandler { service, tool_router, progress_forwarder_spawned }` — *struct*

Implements `rmcp::ServerHandler`. Holds `Arc<SysmlService>`, the generated `ToolRouter<Self>`, and an `AtomicBool` guard so the progress forwarder spawns at most once even across re-init handshakes. Built via `SysmlMcpHandler::new(service)`.

#### `Request structs — LoadSourceRequest, FindRequest, FilePathRequest, … (18 typed)` — *structs*

Typed parameter structs deriving `schemars::JsonSchema` (via the `rmcp::schemars` re-export) so the MCP schema carries per-field descriptions. Used by the early/most-trafficked tools. Remaining tools use the generic `JsonParams` pass-through.

#### `pub struct JsonParams { #[serde(flatten)] params: HashMap<String, Value> }` — *struct*

Generic flatten pass-through used by tools that delegate 1:1 to a service command without a hand-written request struct (sessions / batch / orchestrate / views families and more). Trade-off: agents see an opaque object with no per-field schema descriptions — see the param-schema debt note below.

#### `fn dispatch_to_service / dispatch_to_service_with_readiness` — *fn*

The two bridge helpers. Both call `sysml_service::execute_command`; the readiness variant additionally attaches a `_readiness` field (additive, non-breaking) keyed off `SysmlService::readiness_for(uri)`.

#### `impl ServerHandler — get_info / on_initialized` — *impl*

`get_info` advertises the `tools` and `logging` capabilities and ships agent instructions. `on_initialized` spawns (once) a task subscribing to `service.subscribe_progress()` and forwards each `ProgressEvent` as an MCP `notifications/message` log entry (logger `sysml_mcp.progress`).

## Source layout

The crate is two files. All tools live in `lib.rs` (~1.9k lines).

filter

| File | Responsibility | Key items |
|---|---|---|
| src/lib.rs | All 125 `#[tool]` handlers, request structs, dispatch bridge, ServerHandler impl, coverage test | SysmlMcpHandler, serve, dispatch_to_service(_with_readiness), JsonParams, all_commands_have_mcp_tools |
| src/main.rs | Binary entry: stderr-only tracing, build empty service, spawn shared session reaper, call serve() | main, SysmlService::empty(), session_reaper::spawn_session_reaper |

## Usage from an agent

Typical exploration flow once the server is registered (tool calls shown as `name(args)`):

```
// 1. load a model
sysml_load_source({ uri: "demo.sysml", source: "package P { part def Car; }" })

// 2. discover what's loaded — start cheap
sysml_query({ projection: "count" })          // tool injects limit:100 when omitted
sysml_query({ projection: "ids", kind: "PartDefinition" })

// 3. hydrate a specific element
sysml_element({ id: "" })

// 4. check the live tool surface at any time
sysml_command_catalog()

// 5. while a workspace indexes, inspect the readiness envelope
sysml_diagnostics({ uri: "demo.sysml" })       // response carries `_readiness`
sysml_readiness({ uri: "demo.sysml" })         // poll until project.state == "indexed"
```

## Dependencies

**Direct.**

- `sysml-service` — all domain logic; this crate only adapts it

- `sysml-core` (feature `serde`) — model types in responses

- `sysml-id` — ElementId etc.

- `rmcp` — MCP framework (`tool_router`, `tool_handler`, `ServiceExt`, stdio transport)

- `tokio` — async runtime

- `serde` / `serde_json` — request/response JSON

- `tracing` / `tracing-subscriber` — stderr logging

**Notes.**

- **No downstream crates.** This is a binary leaf — nothing in the workspace depends on it.

- `tempfile` is a dev-dependency (readiness-envelope integration test).

- The MCP transport is the *only* public surface; `serve()` is the one exported function.

## Invariants & pitfalls

**Pure transport adapter.**

No domain logic. Every tool calls `dispatch_to_service`; the sole exception is `command_catalog`, which reads the static `SysmlService::command_catalog()`.

**schemars version split.**

`rmcp` re-exports `schemars` 1.x; the workspace uses 0.8. Request structs MUST derive via the `rmcp::schemars` re-export — never the workspace `schemars` — or they won't compile.

**stderr only.**

stdout carries the JSON-RPC stream. `main.rs` pins `tracing_subscriber` to stderr; never `println!`.

**100% coverage test.**

`all_commands_have_mcp_tools` fails if a new `#[service_command]` lacks a matching tool. It asserts service→tool only — it does *not* check descriptions or param schemas, so doc-string rot can slip through.

**Errors never bubble.**

Service errors become `CallToolResult::error(...)` text the agent reads — not protocol failures.

**Progress can lag.**

The forwarder broadcast has capacity 256; on lag it logs a warning and drops events. A client may miss a "Ready" notification and should fall back to polling `sysml_readiness`.

> ⚠  **Known param-schema debt.** ~18 tools use typed request structs (rich MCP schemas); the majority use the generic `JsonParams` pass-through, which exposes an opaque object with no per-field descriptions. Newer session / batch / orchestrate / views tools therefore have weaker discoverability than the early typed ones. Migrating them to typed structs is tracked as modernisation work.

## Testing

```
cargo test -p sysml-mcp
```

- `all_commands_have_mcp_tools` (unit) — builds the `ToolRouter`, then asserts every `registered_command_metas()` entry has a tool with dots→underscores.

- `tests/readiness_envelope.rs` (integration) — loads a tempdir workspace, then asserts `sysml_diagnostics` returns `_readiness.project.state == "indexed"`. Exercises the in-process dispatch helper (the single source of truth shared by every handler), not a live stdio transport.

Part of the [sysml-rs](../../../README.md) workspace · agent guidance in `CLAUDE.md` · regenerated 2026-06-03
