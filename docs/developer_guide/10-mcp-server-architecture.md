# MCP Server Architecture

`sysml-mcp` is the Model Context Protocol transport for `SysmlService`. It exposes the unified command surface to AI agents (Claude Code, Cursor, etc.) over stdio JSON-RPC.

> `sysml-mcp` is a thin transport adapter: 172 `#[tool]` methods in `crates/tooling/sysml-mcp/src/lib.rs` wrap the 171-command `#[service_command]` surface one-for-one, plus `sysml_command_catalog`, which exposes the registry itself and has no service command behind it. 100% command coverage is enforced by CI (`all_commands_have_mcp_tools`). Both counts move whenever a command is added — see [00-architecture.md](00-architecture.md) for how to read the live number.

## Why MCP

SysML v2 is a design language, not a programming language. The interesting questions aren't "find this function" but "which requirements have no verification?", "what breaks if I change this port?", "does this state machine deadlock?". These need semantic understanding of a resolved model graph — something no text-based tool can provide.

`sysml-mcp` exposes the full model intelligence to any MCP client: Claude Code, Cursor, custom agents, CI pipelines. The agent can reason about the design, not just the text.

## Architecture: thin transport over `SysmlService`

```
stdio (JSON-RPC)
  │
  ▼
┌─────────────────────────────────────────┐
│  rmcp transport (stdio)                 │
│    └─ SysmlMcpHandler                   │
│         ├─ tool_router (~38 #[tool])   │
│         └─ Arc<SysmlService>            │
└────────────────┬────────────────────────┘
                 │ dispatch_to_service
                 ▼
       sysml_service::execute_command
       (inventory lookup, JSON in / JSON out)
                 │
                 ▼
        SysmlService::<command>(...)
                 │
                 ▼
        sysml-ide-db / sysml-runtime / …
```

Every tool handler is one line of real work: `dispatch_to_service(&self.service, "sysml.<command>", json!({...}))`. All domain logic stays in `sysml-service`. The MCP crate owns three things:

1. The rmcp protocol surface (tools, schemas, transport).
2. The tool-name → command-name mapping (mechanical: dots → underscores).
3. Optional typed request structs for tools that benefit from explicit MCP schema documentation.

One rule governs this layering: transports are thin, the service owns the model.

## Tool surface (grouped by category)

The full mapping lives in `crates/tooling/sysml-mcp/src/lib.rs`. The `all_commands_have_mcp_tools` test guarantees every `#[service_command]` has a matching MCP tool. Headline categories:

| Category | Representative tools | Backing service commands |
|----------|----------------------|--------------------------|
| Load | `sysml_load_source`, `sysml_load_file`, `sysml_load_file_ts` | `sysml.load_source`, `sysml.load_file`, `sysml.load_file_ts` |
| Query | `sysml_find`, `sysml_element`, `sysml_children`, `sysml_ancestors`, `sysml_descendants`, `sysml_stats`, `sysml_model_tree`, `sysml_loaded_uris` | `sysml.find`, `sysml.element`, … |
| Analysis | `sysml_diagnostics`, `sysml_constraint_check`, `sysml_expression_eval`, `sysml_unverified`, `sysml_trace_matrix`, `sysml_inspect` (X6) | `sysml.diagnostics`, `sysml.constraint.check`, `sysml.inspect`, … |
| Simulation | `sysml_simulate_start`, `sysml_simulate_step`, `sysml_simulate_stop` | `sysml.simulate.*` |
| Action | `sysml_action_start`, `sysml_action_step`, `sysml_action_run` | `sysml.action.*` |
| Orchestrator | `sysml_orchestrate_start`, `sysml_orchestrate_step`, `sysml_orchestrate_inject` | `sysml.orchestrate.*` |
| Verification | `sysml_verify`, `sysml_analysis`, `sysml_solve`, `sysml_trace`, `sysml_flow_inspect` | `sysml.verify`, … |
| Evaluation | `sysml_evaluate`, `sysml_evaluate_constraints`, `sysml_evaluate_verification_cases`, `sysml_evaluate_analysis_cases`, `sysml_evaluate_calculations` | `sysml.evaluate.*` |
| Export | `sysml_export_json`, `sysml_export_plantuml`, `sysml_diagram` | `sysml.export.*`, `sysml.diagram` |
| Store | `sysml_store_save`, `sysml_store_load`, `sysml_store_latest`, `sysml_store_projects`, `sysml_store_history` | `sysml.store.*` |
| What-if | `sysml_whatif`, `sysml_whatif_sweep` | `sysml.whatif`, `sysml.whatif.sweep` |
| Aggregation | `sysml_aggregate` | `sysml.aggregate` |
| Workspace | `sysml_workspace_verify` | `sysml.workspace.verify` |
| Parse | `sysml_parse` | `sysml.parse` |
| Meta | `sysml_command_catalog` | calls `SysmlService::command_catalog()` directly |

The Meta category is the only exception to the "every tool dispatches via inventory" rule — the catalog endpoint introspects the registry itself.

## How tools are declared

The crate uses `rmcp`'s `#[tool_router]` and `#[tool_handler]` macros. Two styles, both supported:

### Typed request (good MCP schema documentation)

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FindRequest {
    /// Workspace URI to search.
    pub uri: String,
    /// Name pattern (substring match).
    pub pattern: String,
    /// Optional kind filter.
    pub kind: Option<String>,
}

#[rmcp::tool_router]
impl SysmlMcpHandler {
    #[tool(name = "sysml_find")]
    async fn find(&self, args: FindRequest) -> CallToolResult {
        dispatch_to_service(&self.service, "sysml.find", json!({
            "uri": args.uri, "pattern": args.pattern, "kind": args.kind,
        }))
    }
}
```

### `JsonParams` pass-through (mechanical, schemaless)

```rust
#[tool(name = "sysml_some_new_command")]
async fn some_new_command(&self, params: JsonParams) -> CallToolResult {
    dispatch_to_service(&self.service, "sysml.some.new.command", params.0)
}
```

Use the typed style for tools with stable, documented parameters; use `JsonParams` for one-to-one pass-throughs where the service command's `#[service_command]` annotation already documents the shape. Both pass through the inventory dispatcher.

## Adding a new MCP tool

1. Add a `#[service_command]` to `sysml-service/src/lib.rs` (inside the `#[service_impl]` block).
2. In `sysml-mcp/src/lib.rs`, add a method to `#[rmcp::tool_router] impl SysmlMcpHandler`:
   - Pick typed-request or `JsonParams` style.
   - Annotate with `#[tool(name = "sysml_<command_with_underscores>")]`.
   - Call `dispatch_to_service(&self.service, "sysml.<command.with.dots>", json!({...}))`.
3. `cargo test -p sysml-mcp` — `all_commands_have_mcp_tools` must stay green.

## Transports

- **stdio** (primary) — Claude Code, VS Code MCP extension, any MCP-aware client. `cargo run -p sysml-mcp` or `sysml-mcp` binary directly.
- **Embedded HTTP+MCP** — `sysml-api --mcp` runs **the same `SysmlService` instance** behind both the REST API and the MCP server. This is the canonical dev path: one process, two transports, shared model state. See `crates/tooling/sysml-api/src/`.

`capabilities.tools` must be set in `get_info()` or MCP clients won't discover the tools — that's the kind of footgun the inventory-driven coverage test catches indirectly (a tool added to the router without capability advertisement still trips on schema export).

## Lifecycle

```
Start
  → Read MCP `initialize` from stdin
  → Build SysmlService (with workspace if --workspace provided)
  → Register tools via #[tool_router]
  → Send `initialized` notification
  → Enter message loop

On tool call:
  → Deserialize args (typed struct or JsonParams)
  → dispatch_to_service(service, "sysml.<cmd>", json_args)
    → execute_command in sysml-service (inventory lookup)
    → SysmlService::<method>(...) returns Result<T, ServiceError>
  → Wrap success as CallToolResult::structured(...), error as CallToolResult::error(...)
  → Write response to stdout

On stdin EOF / `shutdown`:
  → Stop tool router, drop SysmlService, exit cleanly
```

## Key design patterns / invariants

1. **Pure transport adapter.** No domain logic in this crate. Every tool delegates to `sysml-service` via `dispatch_to_service`. The single exception (`sysml_command_catalog`) reads a static registry, not domain state.
2. **`rmcp::schemars`, not workspace `schemars`.** rmcp re-exports `schemars` 1.x; the workspace uses 0.8. All request structs must derive `schemars::JsonSchema` using the `rmcp::schemars` re-export (imported at the top of `lib.rs`). Mixing them is a compile error.
3. **Underscore tool names, dotted command names.** `sysml_load_source` ↔ `sysml.load_source`. Mechanical translation. Coverage gate enforces the table is complete.
4. **Errors wrap as `CallToolResult::error`, never bubble.** Tools never return `Err` at the MCP layer. The AI agent sees a text error message rather than a protocol-level failure.
5. **stdout is the transport.** All logging (`tracing_subscriber`) goes to **stderr**. Any `println!` or stdout write corrupts the JSON-RPC stream — this is the #1 cause of mysterious "MCP server disappeared" failures.
6. **100% coverage enforced.** `all_commands_have_mcp_tools` (in `crates/tooling/sysml-mcp/tests/...`) builds the `ToolRouter`, collects all tool names, and verifies every entry from `sysml_service::registered_command_metas()` has a matching tool. Adding a `#[service_command]` without a tool fails CI.

## MCP resources (planned, not yet shipped)

The original design proposed resources alongside tools (e.g., `sysml://project/{id}/graph` as a subscribable resource that updates on file change). Tools cover all current use cases; resources remain a future addition if AI clients start preferring subscribe-then-poll patterns over per-call queries.

## MCP prompts (planned, not yet shipped)

Pre-built agent workflows (design review, impact analysis, test generation) were also part of the original proposal. Today these are composed by the calling agent across multiple tool calls; baking them in as MCP prompts is on the backlog.

## Common pitfalls

- **Forgetting the MCP tool when adding a service command.** Coverage test catches this — fix by mirroring an existing tool with `JsonParams` pass-through.
- **Logging to stdout.** Will silently corrupt JSON-RPC frames. Always `tracing_subscriber::fmt().with_writer(io::stderr)…`.
- **HashMap key ordering in overrides.** `ConstraintCheckRequest` and `ExpressionEvalRequest` convert `HashMap` to `Vec<(String, String)>` before dispatch — the service expects an array of tuples, not a map. If you add a new tool with overrides, mirror that conversion.
- **Restart after backend changes.** When testing live, rebuild + relaunch `sysml-api --mcp` (the canonical embedded transport). The detached `sleep infinity | sysml-api --mcp` pattern keeps it running across reloads.

## Testing

```bash
cargo test -p sysml-mcp       # Coverage gate: all_commands_have_mcp_tools
cargo test -p sysml-service   # Backing service tests
cargo test -p sysml-api -- --test-threads=1  # Embedded HTTP+MCP transport
```

No live-MCP integration test currently runs in CI; the coverage gate plus service tests are the contract.

## Where each piece lives

| Concern | File |
|---------|------|
| Tool router, `SysmlMcpHandler` | `crates/tooling/sysml-mcp/src/lib.rs` |
| Standalone stdio binary | `crates/tooling/sysml-mcp/src/main.rs` |
| Embedded HTTP+MCP transport | `crates/tooling/sysml-api/src/` (`--mcp` flag) |
| `dispatch_to_service` bridge | `crates/tooling/sysml-mcp/src/lib.rs` |
| Coverage gate | `crates/tooling/sysml-mcp/tests/...` |
| Backing service commands | `crates/tooling/sysml-service/src/lib.rs` |

## Related documentation

- [00-architecture.md](00-architecture.md) — overall layering.
- [11-sysml-service-design.md](11-sysml-service-design.md) — the service this transport adapts.
- [07-lsp-architecture.md](07-lsp-architecture.md) — sibling stdio transport (LSP).
- `crates/tooling/sysml-mcp/README.md` — quick-reference module map + pitfalls.
- The rule that transport adapters hold no domain state applies identically here.
