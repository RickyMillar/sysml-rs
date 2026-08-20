# sysml-service

The unified service hub: the single owner of SysML v2 domain state, and the one place every transport (CLI, LSP, REST, MCP) dispatches through. Salsa-backed, session-aware, 124 registered commands.

`Layer 5 · tooling` · `unified service hub` · `crate-type: rlib` · `124 commands` · `salsa-backed`

## What it owns

`sysml-service` is the hub of the tooling layer. Below it sit the language and incremental-analysis crates; above it sit the four transports. Its charter is to be the **single source of truth for domain state and operations**: every command a user can run — from `sysml.find` to `sysml.simulate` — is defined here exactly once and exposed identically to all transports.

**One state owner.**

Model state lives in a salsa-backed `AnalysisHost` behind a `std::sync::Mutex`, not in ad-hoc per-file maps. Queries (`parse_file`, `elaborate_workspace`) are memoized and only re-run when an input file, the library, or the project file set changes.

**One command surface.**

Commands are declared with `#[service_command]` inside a `#[service_impl]` block. Each annotation auto-generates a request struct, a `CommandMeta`, a `ServiceCommand` impl, and an `inventory::submit!` registration. The inventory is the single registry.

**One open path.**

`open_context(OpenTarget)` is the single entry for opening a file, folder, or synthetic workspace. CLI / LSP / MCP all route through it — there is no transport-specific file loader. Mode resolution (Strict / Discovered / DiscoveredViaManifest) is delegated to `sysml-project`.

**One session model.**

Execution sessions (simulation, action, orchestrator) are unified as `RuntimeSession`, keyed by `ElementId`, with per-kind live-session quotas tracked in an atomic counter array. Completed sessions move to a `SessionArchive`.

## Where it sits

```text
transports sysml-cli sysml-lsp-server sysml-api (REST) sysml-mcp
▼ typed method · or execute_command(json) ▼
hub sysml-service sysml-service-macros
▼
analysis sysml-ide-db (salsa) sysml-query sysml-resolve sysml-store
▼
lang sysml-runtime sysml-diagram sysml-parser-incremental sysml-core sysml-project sysml-manifest
```

>  **Delegation, not reimplementation.** Compilation and execution logic live in `sysml-runtime` (`ModelCompiler`, the `Executor` trait). Cross-file elaboration is a salsa query in `sysml-ide-db`. Row-shaped list reads go through `sysml-query`. The service orchestrates these; it does not duplicate them.

## The `SysmlService` struct

Verified against `src/lib.rs` (struct at line 303). These are the real fields — there is no `graphs` map, no `parser_cache`, no string-keyed `sessions`.

filter

| Field | Type | Role |
|---|---|---|
| `host` | `Arc<Mutex<AnalysisHost>>` | Salsa-backed incremental DB — the canonical model state (ADR-010, S2.T1). All graph reads go through a memoized `Analysis` snapshot. |
| `sessions` | `DashMap<ElementId, RuntimeSession>` | Active runtime sessions, keyed by `ElementId` (serializes as a UUID-shaped string on the wire). |
| `session_counts` | `[AtomicUsize; 3]` | Per-kind live-session counters indexed by `SessionKind::index()`; read on every quota check instead of scanning the map. |
| `batches` | `DashMap<String, Arc<RwLock<BatchSession>>>` | Batch sessions (R5.0): a parent owning N independently addressable child runtime sessions. |
| `archive` | `Arc<dyn SessionArchive>` | Completed-session archive (R4.1); defaults to `InMemorySessionArchive`, overridable via `with_archive`. |
| `query_cache` | `DashMap<String, QueryResult>` | Process-local cache for the unified `sysml.query` primitive, keyed by `(uri, QuerySpec, graph revision)`. |
| `store` | `Option<Arc<RwLock<dyn Store>>>` | Optional persistence backend (in-memory or PostgreSQL via `sysml-store`). |
| `project_registry` | `RwLock<ProjectRegistry>` | Loaded project manifests for source-file I/O. |
| `diagram_manager` | `DiagramManager` | Open-diagram state: view types, expanded nodes, cached graphs. |
| `hover_source_cache` | `RwLock<HashMap<String, Arc<String>>>` | External/library source text cached for hover docs. |
| `progress_bus` | `broadcast::Sender<ProgressEvent>` | Lifecycle-event bus (P-RA4), capacity 256; lagged subscribers drop, never block. |
| `library_lifecycle` | `RwLock<Option<LibraryReadiness>>` | Transient `Loading`/`Failed` override for `Readiness.library` not derivable from the host. |
| `workspace_root` | `Option<PathBuf>` | Set when constructed via `from_workspace`. |

## Command surface

**124** distinct `#[service_command]` commands (verified: distinct `name = "sysml.…"` entries in `src/lib.rs`). Categories are declared on each command via the `CommandCategory` taxonomy in `command_meta.rs`.

**Execution · 69.**

Session-based execution (Tier 3): simulate, step, orchestrate, batch, sessions.* family.

**Query · 35.**

Stateless model reads (Tier 1): find, children, ancestors, descendants, stats — most delegate to `sysml.query`.

**FileManagement · 7.**

Workspace and source loading via `open_context`.

**Storage · 5.**

Persistence (Tier 5): save, load, history, projects.

**Visualization · 4.**

Diagram generation and JSON / PlantUML export (Tier 4).

**Analysis · 4.**

Cached analysis (Tier 2): verification, sensitivity, aggregation.

> **The inventory is the source of truth.** `command_meta::command_count()` and `command_trait::registered_command_metas()` return the live registry at runtime — prefer these over any hardcoded number, which is what makes the count above verifiable rather than guessed.

## How a command is built

One `#[service_command]` annotation expands into four companion items emitted *after* the impl block by the `#[service_impl]` container macro:

```
#[sysml_service_macros::service_impl]
impl SysmlService {
    #[service_command(
        name = "sysml.find",
        category = Query,
        description = "Find elements by name pattern",
        returns = "Vec<Element>",
    )]
    pub fn find(&self, uri: &str, pattern: &str) -> Result<Vec<Element>, ServiceError> {
        // delegates to sysml_query::QuerySpec under the hood
    }
}
// generated: FindRequest (Deserialize) · FIND_META (CommandMeta)
//            FindCommand (impl ServiceCommand) · inventory::submit!
```

### Two ways to call a command

```text
transportCLI / LSP / REST / MCP
▼
nativeservice.find(uri, pat) — or — execute_command(&service, "sysml.find", json)
▼ (json path) inventory lookup → ServiceCommand::execute ▼
resulttyped value · or serde_json::Value
```

## Key modules

The crate has **41** modules. Beyond the dispatch core, it hosts a large body of IDE-feature logic (completion, hover, rename, goto, formatting, code actions) migrated in from the LSP server so that all transports share one implementation.

filter

| Module | Group | Responsibility |
|---|---|---|
| `lib.rs` | core | `SysmlService` struct + all `#[service_command]` methods, graph accessors (`require_graph`, `workspace_aware_graph`, `host_analysis`). |
| `command_meta.rs` | core | `CommandMeta`, `ParamMeta`, `CommandCategory`; `command_count()` / `command_registry_all()`. |
| `command_trait.rs` | core | `ServiceCommand` trait, inventory registration, `execute_command` (JSON dispatch). |
| `error.rs` | core | `ServiceError` enum. |
| `types.rs` | core | Result types + core re-exports. |
| `open_context.rs` | file | `open_context(OpenTarget)` — single entry for file/folder/synthetic opens; materialises `OpenContext` onto the salsa `ProjectFileSet`. |
| `fs.rs` | file | Flat source-file enumeration helpers (used by CLI workspace inspect + test corpora). |
| `project_registry.rs` | file | Loaded project-manifest registry. |
| `project_discovery.rs` | file | Project / workspace discovery glue. |
| `diagnostics.rs` | file | Diagnostic pipeline (parse → resolve → validate → health), incl. strict-mode enrichment. |
| `query.rs` | query | Element queries (find, children, ancestors, descendants, stats) over the query primitive. |
| `model_tree_query.rs` | query | Model-tree navigation queries. |
| `snapshot.rs` | query | `ServiceSnapshot` — immutable read view of a graph. |
| `evaluation.rs` | analysis | Constraint / verification / analysis evaluation on a `ModelGraph`. |
| `whatif.rs` | analysis | What-if parameter analysis. |
| `sensitivity.rs` | analysis | Parameter-sensitivity sweeps. |
| `aggregation.rs` | analysis | Satisfaction-matrix aggregation (pass/fail/inconclusive). |
| `workspace_verify.rs` | analysis | Cross-file workspace verification. |
| `verify_timeline.rs` | analysis | Verification timeline construction. |
| `expression_ast.rs` | analysis | Expression-AST inspection. |
| `bounds.rs` | analysis | Numeric bounds computation. |
| `execution.rs` | execution | `RuntimeSession`, `SessionKind` (3 variants), per-kind `quota_for`, session insert/remove/retain. |
| `batch.rs` | execution | `BatchSession` — parent owning N child runtime sessions. |
| `session_events.rs` | execution | Session lifecycle events. |
| `session_reaper.rs` | execution | Background expiry of idle sessions. |
| `constraint_monitor.rs` | execution | Live constraint monitoring during a session. |
| `completion.rs` | ide (migrated) | Code-completion provider. |
| `hover.rs` | ide (migrated) | Hover docs. |
| `references.rs` | ide (migrated) | Find-references. |
| `rename.rs` | ide (migrated) | Rename / symbol edits. |
| `goto_definition.rs` | ide (migrated) | Go-to-definition. |
| `formatting.rs` | ide (migrated) | Source formatting. |
| `code_actions.rs` | ide (migrated) | Code actions / quick fixes. |
| `inspect.rs` | ide (migrated) | Token / element inspection. |
| `outline.rs` | ide (migrated) | Document outline. |
| `position.rs` | ide (migrated) | Byte-offset ↔ position mapping. |
| `visualization.rs` | visualization | Diagram generation, JSON / PlantUML export. |
| `diagram_manager.rs` | visualization | Diagram view state. |
| `diagram_edit.rs` | visualization | Diagram edit operations. |
| `storage.rs` | storage | Store delegates (save, load, history, projects). |
| `library_cache.rs` | storage | On-disk standard-library cache (format-versioned). |
| `progress.rs` | lifecycle | `ProgressBus`, `ProgressEvent`, `LibraryPhase`; `subscribe_progress` / `publish_progress`. |
| `readiness.rs` | lifecycle | `Readiness` / `LibraryReadiness` — derived from the host plus the lifecycle override. |

## Public API highlights

#### `SysmlService::empty() / with_store / with_archive / from_workspace`

Constructors. All return an owned `SysmlService` (never `Arc<SysmlService>`) — interior state is already thread-safe. `from_workspace(&Path)` additionally records the workspace root and is the typical entry for CLI/REST.

#### `host_arc(&self) -> &Arc<Mutex<AnalysisHost>>`

Exposes the shared salsa host so a transport (e.g. the LSP server) can share the same incremental DB. Internally, sync commands obtain a memoized `Analysis` snapshot via the private `host_analysis()` and drop the lock immediately.

#### `open_context(&self, target: OpenTarget) -> Result<OpenContext, ServiceError>`

The single open path for every transport. Resolves the project mode via `sysml-project`, materialises `OpenContext { project, kind, root, files, library, diagnostics }`, and threads the kind onto the salsa `ProjectFileSet` input.

#### `workspace_aware_graph(&self, uri) / require_graph(&self, uri)`

`workspace_aware_graph` routes through salsa's `elaborate_workspace[_with_library]` (memoized cross-file graph) — use it for anything that compiles via `ModelCompiler` (simulation, verification, ODE). `require_graph` returns the per-file parse-only graph for read-only queries; `__workspace__` delegates to the elaborated graph.

#### `execute_command(&service, name, json) — command_trait.rs`

Type-erased dispatch: looks up the command in the inventory by name, deserializes the JSON body into its generated request struct, and runs `ServiceCommand::execute`. The path every JSON transport (REST/MCP) takes.

#### `subscribe_progress / publish_progress / reset_library_lifecycle — progress.rs`

P-RA4 lifecycle API. Transports subscribe to the broadcast bus and translate `ProgressEvent`s to their native progress idiom (LSP `workDoneProgress`, CLI spinner). Library load phases overlay onto `readiness_for(_).library`.

## Sessions & quotas

One unified `RuntimeSession` type subsumes simulations, action runs, and orchestrators. `SessionKind` has three variants; each has its own concurrent quota (no single global cap):

| SessionKind | Concurrent quota | Counter slot |
|---|---|---|
| `Simulation` | 30 | `session_counts[Simulation.index()]` |
| `Action` | 30 | `session_counts[Action.index()]` |
| `Orchestrator` | 20 | `session_counts[Orchestrator.index()]` |

Quota checks read the atomic counter array (`quota_for(kind)` in `execution.rs`) rather than scanning the sessions map. On stop, a session moves into the `SessionArchive` and stays queryable via the `sysml.sessions.archive.*` command family.


## Usage

```
use sysml_service::SysmlService;
use sysml_service::command_meta;

// Construct the hub (owned, thread-safe).
let service = SysmlService::empty();

// How many commands are registered? Ask the live inventory.
let n = command_meta::command_count();
assert!(n >= 124);

// Typed, Rust-native call.
let elements = service.find("__workspace__", "Vehicle")?;

// Type-erased JSON call — the path REST/MCP take.
use sysml_service::command_trait::execute_command;
let body = serde_json::json!({ "uri": "__workspace__", "pattern": "Vehicle" });
let result = execute_command(&service, "sysml.find", body)?;
# Ok::<(), sysml_service::error::ServiceError>(())
```

## Dependencies

**Upstream (consumed).**

- `sysml-ide-db` — salsa `AnalysisHost` / `Analysis`

- `sysml-runtime` — `ModelCompiler`, `Executor`, montecarlo (full IR + execution + physics)

- `sysml-query` — the `QuerySpec` primitive

- `sysml-resolve`, `sysml-store`, `sysml-project`, `sysml-manifest`

- `sysml-diagram` — server-side diagram rendering (rlib, no wasm)

- `sysml-parser-incremental` (+ `tree-sitter`) — the sole parser

- `sysml-core`, `sysml-id`, `sysml-span`, `sysml-parser-trait`

- `sysml-service-macros` — the `#[service_command]` / `#[service_impl]` proc macros

**Downstream (transports).**

- `sysml-cli` — command-line tool

- `sysml-lsp-server` — LSP protocol

- `sysml-api` — REST (axum)

- `sysml-mcp` — MCP server for AI agents

Each transport dispatches through this crate and adds no domain logic of its own.

## Invariants & pitfalls

- **One state owner.** Model state is the salsa `AnalysisHost` — there is no `graphs` map and no separate parser cache. Read via `host_analysis()`; hold the lock briefly.

- **Tree-sitter is the only parser.** The Pest path is gone; treat any lingering "Pest fallback" comment as dead text, not a live branch.

- **Compilation delegates to runtime.** Use `workspace_aware_graph` + `sysml_runtime::compiler::ModelCompiler`; never inline elaboration in a command.

- **Inventory is canonical.** Adding a `#[service_command]` auto-registers it; the manual registry was deleted.

- **MCP coverage is enforced.** A new command without a matching MCP tool fails `cargo test -p sysml-mcp` (`all_commands_have_mcp_tools`).

- **Prefer the query primitive.** New list/picker reads should go through `sysml.query`; don't add per-kind `*.list` commands for row-shaped results.

## Testing

```
cargo test -p sysml-service           # unit tests in lib.rs + 12 contract suites under tests/
cargo test -p sysml-service-macros    # proc-macro expansion tests
cargo test -p sysml-mcp               # MCP coverage (every command has a tool)
cargo bench -p sysml-service          # bench_check_constraints, bench_sim_start
```

Part of the [sysml-rs](../../../README.md) workspace · regenerated 2026-06-03
