# Architecture & Crate Layering

This is the structural map of the sysml-rs workspace: what the crates are, how
they are layered, and which dependency directions are allowed. Read it before
adding a crate, adding a dependency, or wiring a new transport.

The workspace holds **10 lang crates** (`crates/lang/`), **11 tooling crates**
(`crates/tooling/`), and **1 test-only crate** (`crates/testing/`). Two more
workspace members sit outside that grouping: `tools/spec-index` (regenerates the
derived spec indexes under `references/sysmlv2/derived/`) and
`editors/simulation-app/src-tauri` (the `sysml-desktop` shell, excluded from
`default-members` because roughly a third of the dependency graph exists only
for it).

To check the crate list yourself:

```bash
cargo metadata --no-deps --format-version 1 | jq -r '.packages[].name' | sort
```

That prints 25 names: the 22 above, plus `spec-index`, `sysml-desktop`, and
`tree-sitter-sysml` — the last being the generated grammar package vendored
inside `crates/lang/sysml-parser-incremental/tree-sitter/`.

## Why Layers?

The workspace is split into **layers** — lower layers don't know about higher
layers — and into filesystem groups: `crates/lang/` (the SysML v2 spec
implementation) and `crates/tooling/` (developer tools that consume the lang
crates), with `crates/testing/` for test-only crates.

| Benefit | What It Means |
|---------|---------------|
| **Faster builds** | Change a higher-layer crate? Only rebuild that layer and above. |
| **Easier testing** | Test `sysml-core` without starting a service or HTTP server. |
| **Flexible deployment** | Use just the parser without LSP/REST/MCP. |
| **Clear contracts** | Each layer has a defined interface (often a trait). |
| **No circular deps** | The build always succeeds in one pass. |

## Big-Picture Shape

```
                    ┌──────────────────────────────────────────────┐
                    │  sysml-cli   sysml-api   sysml-mcp   sysml-   │  Layer 5
                    │                                    lsp-server │  Transports
                    └───────────────────────┬──────────────────────┘
                                            │  (sysml-api also embeds
                                            │   lsp-server + mcp)
                    ┌───────────────────────▼──────────────────────┐
                    │                sysml-service                 │  Unified command
                    │      (#[service_command] dispatch — CLI,     │  surface
                    │       LSP, REST and MCP all route here)      │
                    └───────────────────────┬──────────────────────┘
                                            │
              ┌──────────────┬──────────────┼──────────────┬──────────────┐
              ▼              ▼              ▼              ▼              ▼      Layer 4
        ┌───────────┐  ┌──────────┐  ┌───────────┐  ┌───────────┐  ┌───────────┐ Tooling
        │sysml-ide- │  │sysml-    │  │sysml-     │  │sysml-     │  │sysml-     │ infra
        │  db       │  │  query   │  │  store    │  │  resolve  │  │  layout   │
        │ (salsa)   │  │          │  │           │  │           │  │(no current│
        └─────┬─────┘  └────┬─────┘  └─────┬─────┘  └─────┬─────┘  │ consumer) │
              │             │              │              │        └───────────┘
              │             │              │              ▼
              │             │              │      ┌──────────────────┐
              │             │              │      │ sysml-project /  │
              │             │              │      │ sysml-manifest   │
              │             │              │      └──────────────────┘
              ▼             │              │                                     Layer 3
    ┌───────────────────────┴──────────────┴────┐                                Lang
    │      sysml-runtime      sysml-diagram     │                                features
    │  (execution + analysis IR)  (view models) │
    └─────────────────────────┬─────────────────┘
                              │
              ┌───────────────┴───────────────┐                                  Layer 2
              │  sysml-parser-incremental     │                                  Text
              │  (tree-sitter) — implements   │                                  frontend
              │  sysml-parser-trait::Parser   │
              └───────────────┬───────────────┘
                              ▼
                   ┌─────────────────────┐                                       Layer 1
                   │     sysml-core      │  Semantic model
                   │ (Element, Relation, │  (ModelGraph is the universal IR)
                   │  ModelGraph)        │
                   └─────────┬───────────┘
                             │
              ┌──────────────┼──────────────┬──────────────┐                     Layer 0
              ▼              ▼              ▼              ▼                     Foundations
        ┌──────────┐   ┌──────────┐   ┌──────────┐   ┌──────────────┐
        │ sysml-id │   │sysml-span│   │  sysml-  │   │    sysml-    │
        │          │   │          │   │ manifest │   │   project    │
        └──────────┘   └──────────┘   └──────────┘   └──────────────┘
```

**The Rule:** each layer depends only on layers below it — never above. Sideways
dependencies between siblings exist only where there's an explicit contract:
`sysml-parser-incremental` implements `sysml-parser-trait::Parser`;
`sysml-diagram` builds on `sysml-runtime`; `sysml-api` embeds `sysml-lsp-server`
and `sysml-mcp` so one process can serve REST, LSP-over-WebSocket, and MCP off a
single `SysmlService`.

## Layer-by-Layer Explanation

### Layer 0: Foundations

**Path:** `crates/lang/`
**Crates:** `sysml-id`, `sysml-span`, `sysml-project`, `sysml-manifest`, `sysml-codegen`

- `sysml-id` — `ElementId`, `QualifiedName`, `ProjectId`, `CommitId` (the canonical identity types, salsa-keyed)
- `sysml-span` — `Span`, `Diagnostic`, severity levels
- `sysml-manifest` — `sysml.toml` / `sysml.lock` parsing; shared `walk_up<F, R>` directory-walk helper
- `sysml-project` — Project / workspace discovery, `.kpar` archive support, `ProjectHandle` (transient session-scope handle, distinct from `sysml-id::ProjectId`), `KparLockFile` (distinct from `sysml-manifest::LockFile`)
- `sysml-codegen` — Build-time codegen library, consumed by `sysml-core/build.rs`. Its directory is `crates/lang/codegen/`; the *package* name is `sysml-codegen`.

**Why at the bottom:** identity, source location, and project discovery are
universal. These crates are small and compile fast.

**Dependencies:** mostly `std` + `thiserror`, with `serde` feature-gated.
`sysml-project` is the exception — it pulls `sysml-manifest`, `walkdir`, `sha2`,
`zip`, and `tree-sitter` / `tree-sitter-sysml` so it can read project metadata
out of source files during discovery. That tree-sitter edge is the one place a
Layer 0 crate touches the parser; treat it as a known wart rather than a
precedent to copy.

> **Name collisions, deliberately disambiguated.** `sysml-id::ProjectId` is the
> canonical salsa-keyed id; `sysml-project::ProjectHandle` is a transient
> registry index. `sysml-manifest::LockFile` parses `sysml.lock`;
> `sysml-project::KparLockFile` is `.kpar` archive metadata. Different concepts —
> the names were split apart precisely so they can't be confused.

---

### Layer 1: Semantic Core

**Path:** `crates/lang/sysml-core/`

The central model database, and the universal IR: nearly everything above this
layer consumes a `ModelGraph`.

| Module | Contents |
|--------|----------|
| `element.rs` | `Element` + impl + Display |
| `relationship.rs` | `RelationshipKind` + `Relationship` + impls |
| `graph.rs` | `ModelGraph` struct + indexes + `merge` / `merge_from_ref` / `rebuild_indexes` / fingerprint |
| `lib.rs` | Module declarations, generated-code modules, `crossrefs`, re-exports |
| `resolution/` | Two-pass name resolution (types, then features) |
| `validation.rs`, `structural_validation.rs`, `semantic_checks/` | Hand-written validators dispatched by generated code |
| `elaborate/` | Additive elaboration pass (derives implicit relationships/properties) |
| `physics/` | ISQ inference, conservation laws, classification |
| `json.rs` | Canonical JSON serialization (feature-gated on `serde`) |
| `import_health.rs` | Model-level import diagnostics |

**Codegen:** `build.rs` generates five files from the spec TTL / XMI / Xtext
sources — the `ElementKind` enum (182 variants), value enums, typed property
accessors, cross-reference tables, and the semantic-validation dispatcher.

These `*.generated.rs` files are **not in the source tree**. They are written
into `OUT_DIR` under `target/` and pulled in with `include!` from
`crates/lang/sysml-core/src/lib.rs`. There is nothing to hand-edit: to change
generated output, change the generator in `crates/lang/codegen/` or the spec
input, then rebuild. See [04-codegen.md](04-codegen.md).

**Dependencies:** `sysml-id`, `sysml-span`, `rustc-hash`, plus `serde` /
`serde_json` / `rayon` behind features.

---

### Layer 2: Text Frontend

**Path:** `crates/lang/`
**Crates:** `sysml-parser-trait`, `sysml-parser-incremental`

Tree-sitter is the **sole parser**. A trait keeps callers parser-agnostic:

- `sysml-parser-trait` — the `Parser` trait (`fn parse(&[SysmlFile]) -> ParseResult`), `Formatter`, `SysmlFile` (the canonical file-input wire type), `ParseResult` (graph + diagnostics, with `into_resolved()` / `into_validated()` chaining), and library-loading helpers (`load_standard_library<P: Parser>` is generic over any `Parser`).
- `sysml-parser-incremental` — the tree-sitter implementation. Incremental, error-tolerant, and the production parser for every consumer. It has two modes:
  - **Default (no features):** CST-only via the `FastParser` trait, with no `sysml-core` dependency. This is the syntax-only path for highlighting, brackets, and folds.
  - **`semantic` feature:** adds `build_model_graph` (CST → `ModelGraph`) and the `Parser` impl. This is the path the LSP, IDE, service, CLI, and library loading all take.
  - A third `codegen` feature gates two developer binaries, `generate_ts_tokens` and `validate_ts_coverage`, used when regenerating grammar keyword tables.

**`SysmlFile` is canonical:** it is defined in `sysml-parser-trait`.
`sysml-parser-incremental` re-exports it under the `semantic` feature and keeps
a structurally-identical local fallback for the no-semantic build. The cfg-gated
re-export is deliberate: `sysml-parser-trait` depends on `sysml-core`, so a flat
re-export would break the CST-only build's layering invariant.

**Dependencies:**
- `sysml-parser-trait` → `sysml-core`, `sysml-span`
- `sysml-parser-incremental` → `sysml-span`, `tree-sitter`, `tree-sitter-sysml`; under `semantic`, also `sysml-parser-trait` + `sysml-core`

See [02-parsing.md](02-parsing.md) for the grammar and the parse pipeline, and
[ADR-014](../design/adr/014-tree-sitter-canonical-parser.md) for why there is
only one parser.

---

### Layer 3: Lang Features

**Path:** `crates/lang/`
**Crates:** `sysml-runtime`, `sysml-diagram`

- `sysml-runtime` — the execution engine **and the full analysis IR**. Submodules: `expressions` (evaluator — the keystone the runners build on), `actions` (token-flow control-flow graphs), `statemachine` (compilation + execution), `flows` (data flow), `constraints` (compile + evaluate), `cases` (verification / use case / analysis case), `compiler` (`ModelCompiler`), `solvers` + `ode*` + `physics/` (DAE solving via `diffsol`, hybrid continuous/discrete executor), plus `orchestrator`, `scheduler`, `montecarlo`, and `timeseries`.
- `sysml-diagram` — visualisation IR. It produces two wire formats: **`ViewModel`**, the renderer-agnostic scene + design tokens that the React-SVG frontend consumes, and **`SModel`**, the legacy Sprotty-compatible SGraph format still emitted by CLI/LSP/REST/MCP until those callers migrate. It also builds PlantUML text export, tree and table models, and the sim / diagnostic / verdict overlays. It is **rlib-only** — it depends on the full `sysml-runtime` (and therefore `diffsol`, which does not build for `wasm32`), so diagrams are **server-rendered** and delivered to the editor rather than computed in the browser.

**Dependencies:** `sysml-runtime` → `sysml-core`, `sysml-span`, `diffsol`.
`sysml-diagram` → `sysml-core`, `sysml-runtime`, `sysml-span`.

> **Watch the dev-dependency edges.** `sysml-runtime`'s *library* does not depend
> on any parser — it takes a `ModelGraph` it is handed. Its tests and benches do
> pull `sysml-parser-incremental` (semantic) and `sysml-ide-db` as
> dev-dependencies, which means there is a deliberate dev-only cycle with
> `sysml-ide-db`. Cargo permits this because dev-dependencies apply only when
> building this crate's own test/bench targets. Don't "fix" it by promoting
> either edge to a real dependency.

See [06-execution.md](06-execution.md) for what "running" a SysML model means.

---

### Layer 4: Tooling Infrastructure

**Path:** `crates/tooling/`
**Crates:** `sysml-resolve`, `sysml-query`, `sysml-ide-db`, `sysml-store`

| Crate | Role |
|-------|------|
| `sysml-resolve` | Dependency resolution for multi-package projects (path, git, kpar, registry providers) |
| `sysml-query` | Transport-agnostic structured-query engine: the caller supplies a `ModelGraph` + `QuerySpec` and gets back a paged `QueryResult`. No index — full scan per query. Workspace selection, auth, and caching belong to the layers above. |
| `sysml-ide-db` | Salsa incremental database. `AnalysisHost` (mutable input surface) hands out `Analysis` (immutable snapshots for concurrent reads). Caches parse → resolve → validate → elaborate, plus physics, diagram, and view-model queries. Follows rust-analyzer's pattern. |
| `sysml-store` | Persistence. `Store` trait with `InMemory` and PostgreSQL backends, the latter behind the `postgres` feature (`sqlx` + `tokio`). |

> **Retired:** `sysml-layout`, an orthogonal edge-routing engine built as both
> `rlib` and WASM `cdylib`, was deleted on 2026-08-13 (OS-D2 decision 4). It was
> written for the browser diagram package that has since been removed, and no
> workspace crate ever depended on it; the React-SVG renderer does placement and
> orthogonal routing in one elkjs pass instead. The crate is recoverable from
> git history.

**The ide-db pattern:**

```
AnalysisHost (mutable — owned by the LSP / service main loop)
    ├── set_file_content() / set_file_content_in_project()   → salsa input invalidation
    ├── set_overlay() / clear_overlay()                      → unsaved editor buffers
    ├── load_project() / add_project_file_set()
    └── analysis() → Analysis (immutable snapshot)
                       ├── parse_file()                (Layer 2)
                       ├── resolve_file_best()         (sysml-core::resolution)
                       ├── validate_file_best()        (sysml-core::validation)
                       ├── elaborate_file_best() / elaborate_workspace[_with_library]()
                       ├── semantic_tokens() / outline() / document_symbols()
                       └── workspace_name_index() / workspace_ref_index() / …
```

The `_best` suffix marks queries that return the most complete result available
rather than failing when an upstream stage produced diagnostics.

Physics, diagram, and view-model queries are **free salsa functions** over
`&dyn Db` (in `physics.rs`, `diagram.rs`, `view_model.rs`) rather than methods on
`Analysis` — e.g. `workspace_physics_health_best`, `workspace_diagram_best`,
`workspace_view_model_best`.

---

### Layer 5: Service & Transports

**Path:** `crates/tooling/`
**Crates:** `sysml-service`, `sysml-service-macros`, `sysml-lsp-server`, `sysml-cli`, `sysml-api`, `sysml-mcp`

#### sysml-service — the unified command surface

`SysmlService` owns the model state (an `Arc<Mutex<AnalysisHost>>`) plus session,
batch, store, and diagram state. Every transport routes through it, so the same
operation behaves identically over CLI, LSP, REST, and MCP. Commands are declared
with the `#[service_command(name = "…", category = …)]` proc-macro from
`sysml-service-macros`, which registers each one across all four transports
automatically.

The exact command count moves whenever a command is added, so this document does
not pin it. The runtime registry is the authority —
`sysml_service::command_meta::command_count()`, backed by
`registered_command_metas()`. To read it without writing code, ask the REST
transport:

```bash
cargo run --release -p sysml-api -- 127.0.0.1:8137 &
curl -s http://127.0.0.1:8137/commands | jq 'length'    # 171 at the time of writing
```

Beware the shortcut `rg -o 'name = "sysml\.[a-z0-9_.]+"'` — it undercounts,
because two commands (`sysml.timeline.getTrace` and `sysml.timeline.getSnapshot`)
carry capital letters and don't match a lowercase-only pattern.

Two further points about the surface:

- Non-command Rust entry points exist too (for example `SysmlService::load_workspace_source(uri, source)`, for direct callers that need workspace-aware file attribution). These are deliberately **not** `#[service_command]`, so they don't appear in the CLI or MCP catalogs.
- The MCP coverage gate (`all_commands_have_mcp_tools`, in `sysml-mcp`) fails CI if a `#[service_command]` has no matching MCP tool.

#### Transports

- `sysml-lsp-server` — tower-lsp over stdio. A thin wrapper that translates LSP protocol calls into `SysmlService` dispatches; see [07-lsp-architecture.md](07-lsp-architecture.md) and [ADR-010](../design/adr/010-lsp-as-thin-wrapper.md).
- `sysml-cli` — the `sysml` binary. 26 subcommands (`check`, `inspect`, `query`, `export`, `simulate`, `run`, `verify`, `trade-study`, the `init`/`add`/`lock`/`fetch` project family, …). Subcommands delegate to service commands; the CLI keeps only I/O and output formatting. `sysml serve` is gated behind the `server` feature, which pulls in `sysml-api`.
- `sysml-api` — axum REST + WebSocket, and the widest-reach binary: it embeds `sysml-lsp-server` (LSP over WebSocket, the Monaco transport — see [ADR-013](../design/adr/013-monaco-editor-transport.md)) and `sysml-mcp` (with `--mcp`), all sharing one `SysmlService`.
- `sysml-mcp` — an `rmcp` MCP server exposing service commands as MCP tools for AI agents. Runs standalone or inside `sysml-api --mcp`. It carries one tool more than there are service commands: `sysml_command_catalog` exposes the registry itself and has no `#[service_command]` behind it.

**Dependencies:**
- `sysml-service` → `sysml-ide-db`, `sysml-store`, `sysml-resolve`, `sysml-query`, `sysml-runtime`, `sysml-diagram`, `sysml-parser-incremental`, `sysml-service-macros`
- `sysml-lsp-server` → `sysml-service` + `sysml-ide-db` + `tower-lsp`
- `sysml-cli` → `sysml-service` + `clap` (+ `sysml-api` under `server`)
- `sysml-api` → `sysml-service` + `sysml-lsp-server` + `sysml-mcp` + `axum`
- `sysml-mcp` → `sysml-service` + `rmcp`

See [20-sysml-service-design.md](20-sysml-service-design.md) for the macro,
dispatch chain, and the invariants to preserve when adding a command.

---

### Testing

**Path:** `crates/testing/`
**Crate:** `sysml-spec-tests`

Not just parser coverage: this crate holds the workspace's conformance and gate
suites — 43 integration test targets covering spec conformance per
construct family (actions, constraints, state machines, requirements,
verification verdicts, quantities, occurrences), identity invariants,
cross-transport identity baselines, corpus regression, performance baselines,
the language-pack and retrieval evaluations, the derived-index regeneration
gates, and the IP leakage scan.

Cross-parser equivalence gates were retired along with the second parser — with a
single parser there is nothing to diff against. Snapshot tests now lock
tree-sitter parse/resolve output directly.

---

## External Dependency Policy

Keep externals scoped to the layer that needs them. Current allocation:

| Dependency | Allowed In | Why |
|------------|------------|-----|
| `serde` | All crates | Standard serialization (feature-gated in Layers 0–1) |
| `serde_json` | `sysml-core` (feature-gated), `sysml-project`, `sysml-diagram`, and the tooling layers | JSON output / wire format |
| `uuid` | `sysml-id` (feature-gated) | Unique identifiers |
| `salsa` | `sysml-ide-db`, and `sysml-service` (which holds the host) | Incremental computation engine |
| `tower-lsp` | `sysml-lsp-server`, and `sysml-api` (LSP over WebSocket) | LSP protocol implementation |
| `axum` | `sysml-api` only | HTTP / WS server |
| `rmcp` | `sysml-mcp` only | MCP server framework |
| `tokio` | All Layer-5 crates plus `sysml-store` (feature-gated) | Async runtime |
| `sqlx` | `sysml-store` (feature-gated) | Database access |
| `tree-sitter` | `sysml-parser-incremental`, and `sysml-project` for discovery-time metadata reads | Incremental CST parser (the sole parser) |
| `diffsol` | `sysml-runtime` only | DAE/ODE solver for physics execution (blocks `wasm32`) |
| `rayon` | `sysml-core` and `sysml-runtime`, both feature-gated | Data parallelism |
| `regex` | `sysml-codegen`, `sysml-runtime`, `sysml-query` | Pattern matching in codegen and query predicates |
| `thiserror` | All crates | Clean error types |

Verify any row with `cargo tree -i <dep>`.

## Adding New Crates

1. **Identify the layer** — what does it need? what provides it?
2. **Only depend downward** — never on crates above your layer
3. **Decide the filesystem group** — `crates/lang/` (spec implementation) vs `crates/tooling/` (developer tools that consume the lang crates) vs `crates/testing/` (test-only)
4. **Minimise externals** — each new dependency is a build cost
5. **Feature-gate optionals** — don't force users to compile what they don't use
6. **Update this document** — keep the rules visible

## Common Mistakes to Avoid

| Mistake | Why It's Bad | Solution |
|---------|--------------|----------|
| `sysml-parser-incremental` depending on `sysml-core` outside the `semantic` feature | Defeats the fast CST-only IDE path | Gate semantic features; the CST-only path stays a leaf |
| Foundation crates depending on `serde` unconditionally | Forces every consumer to compile serde | Feature-gate it |
| Circular dependencies between sibling crates | Breaks the build | Extract shared types to a lower layer |
| `sysml-core` depending on `sysml-parser-trait` | Inverts the natural flow | The parser produces core, not the reverse |
| A new transport calling `sysml-ide-db` / `sysml-runtime` directly | Reintroduces the bypass paths the service layer exists to remove | Route through `SysmlService` via `#[service_command]` |
| Hand-editing a `*.generated.rs` file | They live in `OUT_DIR` and are overwritten every build | Change the generator in `crates/lang/codegen/` or the spec input |
| Confusing `sysml-id::ProjectId` and `sysml-project::ProjectHandle` | Different concepts | `ProjectId` is the canonical salsa key; `ProjectHandle` is a transient registry index |

## Crates You Will See Referenced But Won't Find

Older commits, branches, and stale `use` statements name crates that no longer
exist. Where they went:

| Old crate | Now lives in | Note |
|-----------|--------------|------|
| `sysml-parser-batch` (Pest PEG) | **deleted** | Tree-sitter (`sysml-parser-incremental`) is the sole parser and implements `Parser`. |
| `sysml-analysis-ir` | `sysml-runtime` | Full analysis IR + execution + physics now live in one crate. |
| `sysml-meta` | `sysml-core` | Metadata / applicability folded in. |
| `sysml-canon` | `sysml-core` | Canonical JSON is `sysml-core::json`. |
| `sysml-store-postgres` | `sysml-store` | PostgreSQL is a backend behind the `postgres` feature. |

## See Also

- `crates/lang/README.md` — cross-cutting lang patterns (codegen, `ModelGraph` as universal IR)
- `crates/tooling/README.md` — cross-cutting tooling patterns
- [02-parsing.md](02-parsing.md) — tree-sitter grammar + parser pipeline
- [03-resolution.md](03-resolution.md) — two-pass name resolution
- [04-codegen.md](04-codegen.md) — build-time spec-to-Rust code generation
- [06-execution.md](06-execution.md) — the execution runtime
- [07-lsp-architecture.md](07-lsp-architecture.md) — LSP as a thin wrapper over the service
- [19-mcp-server-architecture.md](19-mcp-server-architecture.md) — MCP server + AI-agent integration
- [20-sysml-service-design.md](20-sysml-service-design.md) — `#[service_command]`, dispatch, transport adapters
