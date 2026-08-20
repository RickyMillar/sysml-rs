# crates/tooling

The developer-tooling tier of sysml-rs: a unified service hub plus the IDE, query, CLI, storage, and REST surfaces built on top of the [lang/](../lang/README.md) spec crates.

`Layers 3–5 · tooling` · `10 crates` · `69 service commands` · `tree-sitter-only parsing`

## What lives here

Where `crates/lang/` implements the SysML v2 specification (elements, the tree-sitter parser, semantics, runtime, physics), `crates/tooling/` turns that model into something people and agents can *use*. The keystone is `sysml-service`: a single hub that owns all domain state and registers every command exactly once. CLI, LSP, REST, and MCP are thin transports that resolve a command by name and pass it a JSON body — none of them reimplement logic.

>  **Directory note.** `sysml-manifest` and `sysml-parser-incremental` are *not* here — they live in [`crates/lang/`](../lang/README.md). Only `sysml-resolve` carries project-graph concerns inside `tooling/`.

## Layer & dependency map

Arrows point from a consumer to what it depends on. Blue nodes are the service hub; dashed nodes are upstream `lang/` crates.

```text
L5 entry sysml-cli sysml-api sysml-mcp sysml-lsp-server
▼ dispatch commands by name ▼
L3 hub sysml-service ← sysml-service-macros · sysml-query
▼ analysis · storage ▼
L3–4 svc sysml-ide-db sysml-store sysml-resolve
▼ spec model · parser · runtime (crates/lang) ▼
L0–2 lang sysml-core sysml-parser-incremental sysml-runtime sysml-diagram sysml-manifest
```

## Crates

Each crate name links to its own README.

| Crate | Role | Crate type | Description |
|---|---|---|---|
| [`sysml-service`](sysml-service/README.md) | Service hub | lib | Unified service layer — owns all domain state; 69 registered commands dispatched to every transport. |
| `sysml-service-macros` | Proc macro | proc-macro | `#[service_command]` — generates request types, metadata, and `inventory` registrations. |
| [`sysml-mcp`](sysml-mcp/README.md) | Transport | lib + bin | MCP server exposing SysML v2 model intelligence to AI agents (over `sysml-service`). |
| `sysml-query` | Query engine | lib | Transport-neutral structured query engine over model element lists (filter / project / sort). |
| [`sysml-ide-db`](sysml-ide-db/README.md) | Analysis | lib | Salsa-based incremental computation database (`AnalysisHost` / `Analysis`). |
| [`sysml-lsp-server`](sysml-lsp-server/README.md) | Transport | lib + bin | LSP server (tower-lsp, stdio). Largest crate in the workspace (~52 src files). |
| [`sysml-resolve`](sysml-resolve/README.md) | Project | lib | Transitive dependency resolution (path, git, kpar, registry). |
| [`sysml-cli`](sysml-cli/README.md) | Transport | bin | CLI binary: parse, check, simulate, run, inspect, export. `server` feature pulls in `sysml-api`. |
| [`sysml-store`](sysml-store/README.md) | Storage | lib | Store trait + InMemory + PostgreSQL backends (`postgres` feature). |
| [`sysml-api`](sysml-api/README.md) | Transport | lib + bin | REST API server (axum). Routes delegate into `sysml-service`. |

## How a command flows

Every transport ends at the same place. A command name (e.g. `sysml.element`) and a JSON body are handed to `execute_command`, which looks the registration up via `inventory` and invokes its handler against the shared `SysmlService`.

```
// crates/tooling/sysml-service/src/command_trait.rs
pub fn execute_command(
    service: &SysmlService,
    command_name: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, ServiceError> {
    for reg in registered_commands() {
        if reg.meta.name == command_name {
            return (reg.handler)(service, body);
        }
    }
    Err(ServiceError::NotFound(format!("command '{}' not found", command_name)))
}
```

Commands are declared inline on `SysmlService` methods. The proc macro turns each into a typed request, metadata, and an `inventory::submit!` so the registry is assembled at link time — the docs cite the registration count (69 distinct names), never a hand-maintained list.

```
// sketch of a declared command (crates/tooling/sysml-service/src/lib.rs)
#[service_command(name = "sysml.element", /* … */)]
fn element(&self, req: ElementRequest) -> Result<ElementResponse, ServiceError> { /* … */ }
```

## Cross-cutting patterns

**Tree-sitter is the only parser.**

`sysml-parser-incremental` (in `lang/`) implements `sysml_parser_trait::Parser` and returns a real `ModelGraph`-backed result. The old Pest `sysml-parser-batch` is **deleted** — there is no "dual parser" companion. Do not reintroduce one.

**Service owns the state.**

`SysmlService` holds the Salsa host as `Arc<Mutex<AnalysisHost>>` and runtime sessions as `DashMap<ElementId, RuntimeSession>`. There are no `graphs`, `parser_cache`, or `MAX_SESSIONS` fields.

**Salsa AnalysisHost / Analysis.**

Following rust-analyzer: `AnalysisHost` (mutable, owns the DB) yields `Analysis` snapshots (immutable, concurrent). Lock briefly to set inputs or snapshot, then drop. Query results must be `Send + Sync`.

**Feature gates.**

`sysml-store` gates Postgres behind `postgres`; `sysml-cli` gates the REST server behind `server`. Nothing here targets `wasm32`: the one WASM crate, `sysml-layout` (an orthogonal edge router that never had a consumer), was deleted on 2026-08-13 under OS-D2 decision 4 and is recoverable from git history.

## Testing

```
cargo test -p sysml-service                        # service command round-trips
cargo test -p sysml-lsp-server                     # LSP protocol integration
cargo test -p sysml-store --features postgres      # Postgres backend (needs DB)
```

Cross-transport parity — a command producing identical output across CLI/LSP/REST/MCP — is exercised by the fixtures in `crates/testing/sysml-spec-tests`.

Part of the [sysml-rs](../../README.md) workspace · agent guidance in `CLAUDE.md` · regenerated 2026-06-03
