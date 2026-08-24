# SysML Service Layer

`sysml-service` is the unified domain layer. Every transport (CLI, LSP, REST, MCP) dispatches through it. This doc explains why it exists, how `#[service_command]` works, the dispatch chain, and the invariants you must preserve when adding commands.

> What shipped: 171 `#[service_command]` methods, the CLI `inspect` collapse onto a service command, a single `RuntimeSession` for execution, and tree-sitter as the only parser. The command count moves whenever a command is added — `command_count()` is the authority, and [00-architecture.md](00-architecture.md) shows how to read it.

For wider context, see [00-architecture.md](00-architecture.md). For per-crate detail, see `crates/tooling/sysml-service/README.md` and `crates/tooling/sysml-mcp/README.md`.

## Why it exists

Before S2, four transports each maintained their own copy of the same domain state:

- CLI built `EvalContext` ad-hoc in `common.rs`.
- LSP wrapped state in `CommandContext` borrowed from `SysmlLanguageServer`.
- REST had its own `AppState { InMemoryStore }`.
- MCP didn't exist yet — adding it would have meant a fourth copy.

Each repeatedly parsed files, loaded the library, resolved names, elaborated, and managed sessions. New commands required four implementations. Behaviour drifted between transports. The MCP coverage gate landed too late to retroactively detect the drift.

S2 collapsed the four pipelines onto a single `SysmlService`. Transports became thin adapters:

```
     CLI        REST       LSP        MCP
      \          |          |          /
       \         |          |         /
        v        v          v        v
   ┌─────────────────────────────────────┐
   │          SysmlService               │
   │  • AnalysisHost (salsa)             │
   │  • host: Arc<Mutex<AnalysisHost>>   │
   │  • sessions: DashMap<ElementId,Run> │
   │  • project_registry, query_cache,   │
   │    diagram_manager, library_lifecycle│
   │  171 #[service_command] methods     │
   └─────────────────────────────────────┘
          |              |
    ┌─────┴──────┐  ┌───┴────────────┐
    │ sysml-core │  │ sysml-runtime  │
    │ + ide-db   │  │ ModelCompiler  │
    │ (salsa)    │  │ + Executor     │
    └────────────┘  └────────────────┘
```

## What stays in the transport vs. what moves into the service

| Lives in `SysmlService` (domain) | Stays in the transport (protocol/UI) |
|----------------------------------|---------------------------------------|
| `host: Arc<Mutex<AnalysisHost>>` (salsa DB — owns all model graphs) | `Client` (tower-lsp) |
| `query_cache` (paged query results) | `open_diagrams`, `expanded_nodes` (UI view state) |
| `library_lifecycle` (stdlib readiness) | `last_published_diagnostics`, `last_semantic_tokens` (LSP delta) |
| `sessions: DashMap<ElementId, RuntimeSession>` (sim, action, orchestrator) | `pending_requests` (LSP dedup) |
| `hover_source_cache` (per-uri source text) | per-document CST cache (LSP, tree-sitter) |
| `ProjectRegistry` (manifest discovery) | CLI flag parsing |
| `DiagramManager` (canonical diagram state) | HTTP routing, auth, CORS (REST) |
| `Store` (optional persistence) | stdio/SSE transport (MCP) |

## `#[service_command]` — the dispatch surface

Commands are declared with a proc-macro from `sysml-service-macros`, applied inside a `#[service_impl]` block:

```rust
#[sysml_service_macros::service_impl]
impl SysmlService {
    #[service_command(
        name = "sysml.find",
        category = Query,
        description = "Find elements by name pattern",
    )]
    pub fn find(
        &self,
        uri: &str,
        pattern: &str,
        kind: Option<&ElementKind>,
    ) -> Result<Vec<ElementSummary>, ServiceError> { /* ... */ }
}
```

The macro expands into companion items emitted **after** the impl block:

- `FindRequest { uri: String, pattern: String, kind: Option<ElementKind> }` — deserializable from JSON.
- `FIND_META: CommandMeta` — name, category, description, `ParamMeta` list.
- `FindCommand` — `impl ServiceCommand` with a JSON-erased `execute(service, body) -> Value`.
- `inventory::submit!` — registers the command at link time.

Categories: `Query`, `Analysis`, `Execution`, `Visualization`, `Storage`, `Project`, `Diagnostics`. They drive grouping in `sysml.command.catalog` and help docs.

### Dispatch chain

```
Transport (CLI / MCP / REST / LSP)
  │
  │   typed path                                 erased path
  ▼                                              ▼
SysmlService::find(...)                  execute_command(service, "sysml.find", body)
  └─ direct Rust call                       └─ inventory::iter() → ServiceCommand::execute
       returns ElementSummary                    returns serde_json::Value
```

Most call sites use the typed path. The erased path exists for MCP / REST / dynamic CLI subcommands where the command name is a runtime string.

### MCP coverage gate

`sysml-mcp` ships a `cargo test` that fails if any `#[service_command]` lacks a matching MCP tool handler:

```
all_commands_have_mcp_tools  (sysml-mcp/tests/...)
```

This is the load-bearing invariant for "every command works on every transport". Adding a new `#[service_command]` without adding a matching MCP wrapper is a CI failure — usually fixed by mirroring an existing wrapper via `JsonParams` pass-through.

## Direct Rust APIs (not `#[service_command]`)

Not every public service method is a command. Some are session-state setup calls that don't make sense as JSON-RPC endpoints:

| Method | Why it's not a `service_command` |
|--------|----------------------------------|
| `SysmlService::load_workspace_source(uri, source)` | Sets file content + attributes to the workspace project — a stateful Rust API the CLI/LSP use; MCP shouldn't push files arbitrarily |
| `SysmlService::workspace_aware_graph(uri)` | Internal helper used by other commands; not a transport-facing operation |
| Various `pub(crate)` query helpers | Building blocks, not endpoints |

The rule of thumb: if the operation reads/writes the model and returns a transport-neutral result, make it a `#[service_command]`. If it manipulates service internals or is part of a stateful protocol the transports already model differently, keep it a plain Rust method.

## Module map

```
crates/tooling/sysml-service/src/
├── lib.rs                  # SysmlService struct, #[service_impl] block (171 commands)
├── command_meta.rs         # CommandMeta, ParamMeta, CommandCategory
├── command_trait.rs        # ServiceCommand, CommandRegistration, inventory dispatch
├── error.rs                # ServiceError enum
├── types.rs                # Result types + re-exports of core/runtime types
├── snapshot.rs             # ServiceSnapshot — immutable read view of a graph
│
├── query.rs                # find, element, children, ancestors, descendants, stats, …
├── inspect.rs              # X6: collapses CLI inspect onto a service command
├── diagnostics.rs          # Diagnostics pipeline (parse + resolve + validate + elaborate + health)
├── outline.rs              # Document symbols
├── hover.rs                # Hover content
├── completion.rs           # Completion candidates
├── goto_definition.rs      # Goto-def routing
├── references.rs           # Find-refs
├── rename.rs               # Workspace renames
├── code_actions.rs         # Code-action providers
├── formatting.rs           # Source formatting
├── position.rs             # Offset/range conversions
├── model_tree_query.rs     # Model tree shape for the UI
│
├── evaluation.rs           # Constraint / verification / analysis evaluation
├── expression_ast.rs       # Expression AST queries
├── execution.rs            # RuntimeSession (unified), sim/action/orchestrator handlers
├── session_events.rs       # Session event stream
├── session_reaper.rs       # Session expiry
├── batch.rs                # Batch evaluation
├── verify_timeline.rs      # Verification timelines
├── whatif.rs               # What-if + sensitivity sweeps
├── sensitivity.rs          # Sensitivity analysis primitives
├── bounds.rs               # Range/bound computations
├── aggregation.rs          # Satisfaction-matrix aggregation
├── workspace_verify.rs     # Cross-file workspace verification
│
├── visualization.rs        # Diagram ViewModel + JSON / PlantUML export
├── diagram_manager.rs      # Diagram view state
├── diagram_edit.rs         # Diagram-edit operations
│
├── storage.rs              # save / load / history / projects (Store delegate)
├── library_cache.rs        # Stdlib cache
├── project_discovery.rs    # sysml.toml discovery
├── project_registry.rs     # Loaded-manifest registry
├── constraint_monitor.rs   # Live constraint evaluation
└── fs.rs                   # Filesystem helpers
```

## Worked example: the X6 inspect collapse

Before X6, `sysml-cli/src/inspect.rs` re-implemented the full parse → resolve → elaborate → validate → health pipeline twice (single-file ~210 LoC, workspace ~165 LoC). The LSP had its own copy. Drift accumulated.

X6 introduced one `#[service_command]`:

```rust
#[service_command(name = "sysml.inspect", category = Analysis)]
pub fn inspect(
    &self,
    uri: Option<&str>,
    workspace: bool,
    focus_file: Option<&str>,
) -> Result<InspectResponse, ServiceError> { /* ... */ }
```

Internally it routes through the same `compute_full_diagnostics` + `Analysis::semantic_tokens` that the LSP uses. The CLI shrank from ~210/~165 LoC of inline pipeline to thin wrappers (request build → dispatch → format), `--cst` mode kept CLI-local because raw `tree_sitter::Tree` doesn't cross the JSON boundary. Net `inspect.rs` 1170 → 555 LoC.

Two backend fixes were forced by the unification — both *strict improvements* the old per-file pipeline silently masked:

1. **`SysmlService::load_workspace_source(uri, source)`** — direct Rust API, not a `service_command`. Attributes the file to the workspace project via `set_file_content_in_project(uri, source, ProjectHandle(SERVICE_WORKSPACE_PROJECT_ID))` so `compute_full_diagnostics` takes the workspace-resolution path. Without project-id attribution, cross-file imports stayed unresolved.
2. **`diagnostics::compute_pipeline`** — now feeds the workspace-merged graph into `import_health_diagnostics_with_context(graph, library, workspace)` so IM001 ("namespace unresolved in current workspace context") doesn't false-positive on cross-file imports.

This is the canonical S2 pattern: collapse the duplicate, hold the backend honest, gain a free MCP tool.

## Key design patterns / invariants

1. **Thread-safe by construction.** All interior state behind `Arc`, `DashMap`, or `RwLock`. Constructors return owned `SysmlService`, never `Arc<SysmlService>` — let the caller wrap if needed.
2. **`require_graph` vs. `workspace_aware_graph`.** Read-only queries use `require_graph(uri)`. Anything that compiles via `ModelCompiler` (simulation, verification, ODE) uses `workspace_aware_graph(uri)` — that's the variant with cross-file imports resolved.
3. **Compilation delegates to runtime.** Service methods never inline `elaborate()` + `StateMachineCompiler::compile_named()`. They use `sysml_runtime::compiler::ModelCompiler`.
4. **Inventory is the source of truth.** There is no manual `command_registry()`. `registered_command_metas()` returns all commands from `#[service_command]` annotations. New transports MUST iterate this.
5. **Session unification.** One `sessions: DashMap<String, RuntimeSession>` keyed by `"{uri}:{name}"`. Cap is 50 (`execution::MAX_SESSIONS`). Legacy accessors `simulations()` / `action_sessions()` / `orchestrator_sessions()` are backward-compat views over the same map.
6. **Transport-neutral return types.** Every command returns a type that implements `Serialize`. If a runtime type doesn't, the service method serializes to `serde_json::Value`.
7. **`evaluation::EvalConstraintResult ≠ types::ConstraintResult`.** Two structs that look similar but are not interchangeable — the eval one came from migrated LSP logic, the types one is the public service result. Don't confuse them.

## How each transport adapts

### CLI

```rust
// crates/tooling/sysml-cli/src/check.rs
fn run(file: &Path, overrides: &[(String, String)], json: bool) -> Result<()> {
    let service = SysmlService::from_file(file, config)?;
    let results = service.check_constraints(file_uri, overrides)?;
    // … format output
}
```

CLI argument parsing stays in the transport. Everything else delegates.

### LSP

```rust
// crates/tooling/sysml-lsp-server/src/commands.rs
async fn handle_evaluate(service: &SysmlService, ctx: &LspContext, args: &[Value]) -> Value {
    let snapshot = service.snapshot();
    let element = snapshot.element_at(uri, offset)?;
    snapshot.evaluate_element(&element.id)
}
```

`LspContext` shrinks to UI-specific state (`client`, `open_diagrams`, `expanded_nodes`). Domain state lives in `SysmlService`.

### REST

```rust
// crates/tooling/sysml-api/src/lib.rs
async fn handle_command(State(service): State<Arc<SysmlService>>, Path(cmd): Path<String>, Json(body): Json<Value>) -> impl IntoResponse {
    Json(execute_command(&service, &cmd, body))
}
```

The REST shell is mostly an axum router that calls `execute_command(service, cmd_name, body)` — the same erased-dispatch path MCP uses.

### MCP

```rust
// crates/tooling/sysml-mcp/src/lib.rs
fn handle_tool_call(service: &SysmlService, tool: &str, args: Value) -> Value {
    execute_command(service, &tool_to_command_name(tool), args)
}
```

`mcp__sysml__sysml_find` → `sysml.find` is a mechanical name translation. The coverage gate guarantees the table is complete.

## Testing

```bash
cargo test -p sysml-service                       # 76 unit + 1 doc test
cargo test -p sysml-service-macros                # 12 macro tests
cargo test -p sysml-mcp                           # MCP coverage gate
cargo test -p sysml-api -- --test-threads=1       # REST tests (serial for env var isolation)
```

## Common pitfalls

- **MCP coverage failure.** Adding a `#[service_command]` without a matching MCP tool breaks CI. Mirror an existing wrapper in `sysml-mcp/src/lib.rs`.
- **`#[doc = "..."]` on params is metadata, not docs.** The `#[service_impl]` macro strips param-level doc comments from the output — they're only used to populate `ParamMeta`. If you need rustdoc on the param, put it on the method-level doc comment.
- **Don't reintroduce transport bypass paths.** Every new domain feature goes in `sysml-service`. If a transport needs an operation, expose it as a `service_command`, not a transport-local helper.
- **Don't widen `Visualization` to leak renderer-specific types.** Return `serde_json::Value` from diagram endpoints; the frontend renderer owns the shape.

## Related documentation

- [00-architecture.md](00-architecture.md) — how the service sits in the broader layering.
- [07-lsp-architecture.md](07-lsp-architecture.md) — the LSP transport adapter.
- [10-mcp-server-architecture.md](10-mcp-server-architecture.md) — the MCP transport adapter and tool registry.
- [07-lsp-architecture.md](07-lsp-architecture.md) — "LSP holds no domain state", the principle the service surface embodies.
- `crates/tooling/sysml-service/README.md` — quick-reference module map + pitfalls.
- `crates/tooling/sysml-service-macros/src/` — proc-macro implementation.
