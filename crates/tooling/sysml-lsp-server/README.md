# sysml-lsp-server

Language Server Protocol server for SysML v2 — a thin tower-lsp transport that translates editor requests into `SysmlService` command dispatches over a shared salsa `AnalysisHost`.

`Layer 5 · tooling` · `LSP transport` · `crate-type: lib + bin` · `tower-lsp 0.20 · stdio`

## Overview

`sysml-lsp-server` is the top of the IDE stack. It speaks the Language Server Protocol over stdio (via `tower-lsp`) and is, by design, a **thin transport**: LSP requests are translated into `sysml-service` commands and salsa queries against a shared `AnalysisHost`. The same `SysmlService` instance can be reused by the REST `/lsp` WebSocket bridge, so an editor `did_change` and a REST `sysml.*` command observe the same incremental database.

Resolution, execution (simulation / verification / what-if), diagram model generation and the standard-library lifecycle all live *below* this crate. The LSP server owns only protocol plumbing: document sync, diagnostic scheduling/debounce, capability advertisement, command routing, and the UX/telemetry message contract.

>  **Post-cleanup reality.** The deleted `sysml-parser-batch` (Pest) crate is *not* a dependency. Parsing flows through `sysml-parser-incremental` (tree-sitter, the sole parser, implementing `sysml_parser_trait::Parser`). The old in-browser WASM `DiagramEngine` is gone — diagrams are **server-rendered** here and delivered to webviews via the `sysml/diagram/setModel` notification.

## Where it sits

```text
editors VS CodeZed Neovim — LSP / stdio —▸
▾
L5 transport sysml-lsp-server
▾ dispatch commands · salsa queries
L5 hub sysml-service
L4 analysis sysml-ide-db sysml-resolve sysml-runtime sysml-diagram
L1–L3 sysml-parser-incremental sysml-core sysml-project sysml-manifest sysml-span
```

## Public API

The library surface is intentionally tiny — three free functions plus the server type. Everything else is feature logic invoked through the `LanguageServer` trait.

#### `— *pub async fn run_stdio()` — *main entry*

Runs the server on stdin/stdout: builds the service via `create_service()` and serves it with `tower_lsp::Server`. This is what the `sysml-lsp-server` binary (`src/main.rs`) calls after tracing + panic-hook setup.

#### `— *pub fn create_service() -> (LspService<SysmlLanguageServer>, ClientSocket)` — *fn*

Builds a fresh `LspService` with a new `SysmlLanguageServer` (each gets its own `SysmlService` / `AnalysisHost`). Use this for a standalone stdio server.

#### `— *pub fn create_service_with(service: Arc<SysmlService>) -> (LspService<…>, ClientSocket)` — *fn*

Builds an `LspService` that *reuses* an existing `SysmlService`. Used by the REST `/lsp` WebSocket bridge so LSP `did_change` edits and REST `sysml.*` commands share one salsa `AnalysisHost`.

#### `— *pub struct SysmlLanguageServer` — *struct*

The `tower_lsp::LanguageServer` implementation. Constructed via `::new(client)` or `::new_with_service(client, Arc<SysmlService>)`. Holds the `Client` handle and the shared service; all protocol handlers are `impl LanguageServer` blocks in `lib.rs` that delegate to the feature modules.

#### `— *pub mod lsp_types` — *module*

The only re-exported module. Minimal LSP protocol type conversions (formerly the separate `sysml-lsp` crate, merged in) — converts `sysml_span::Diagnostic` to LSP `Diagnostic` form.

### Usage

```
// Binary: run the server on stdin/stdout (this is src/main.rs).
use sysml_lsp_server::run_stdio;

#[tokio::main]
async fn main() {
    run_stdio().await;
}
```

```
// Embedding: reuse one SysmlService across an LSP + REST bridge.
use std::sync::Arc;
use sysml_lsp_server::create_service_with;
use sysml_service::SysmlService;

let service = Arc::new(SysmlService::new());
let (lsp_service, socket) = create_service_with(service.clone());
// `lsp_service` and any REST transport over `service` now share one AnalysisHost.
```

## Advertised capabilities

Declared in `ServerCapabilities` at `lib.rs` (the `initialize` handler). Filter by capability or method.

| Capability | LSP method | Source module | Notes |
|---|---|---|---|
| Text document sync | didOpen/Change/Close | lib.rs | Full sync |
| Diagnostics | publishDiagnostics | diagnostics.rs · diagnostic_pipeline.rs | Debounced (150ms), fingerprinted; syntax + resolution + semantic |
| Document symbols | documentSymbol | symbols.rs | Outline / breadcrumbs |
| Workspace symbols | workspace/symbol | symbols.rs | Scored, ranked |
| Completion | completion (+resolve) | completion.rs · kinds.rs | exact>prefix>fuzzy, snippets, resolve provider |
| Go to definition | definition | navigation.rs | Cross-file via resolution |
| Go to type definition | typeDefinition | navigation.rs |  |
| Find implementations | implementation | navigation.rs | Specialization / typing search |
| References | references | navigation.rs | Graph traversal |
| Hover | hover | hover.rs | Signature, supertypes, inherited members, evaluated values |
| Rename | rename (+prepare) | rename.rs | Validation + cross-file edits |
| Semantic tokens | semanticTokens (full + range) | semantic_tokens.rs | Model + CST based |
| Code actions | codeAction | code_actions.rs | Quick fixes |
| Code lens | codeLens | code_lens.rs | Constraint / verification run lenses |
| Inlay hints | inlayHint | inlay_hints.rs | Disable via `SYSML_LSP_DISABLE_INLAY_HINTS=1` |
| Formatting | formatting | formatting.rs | Document formatting |
| Folding ranges | foldingRange | ranges.rs | Block-level |
| Selection ranges | selectionRange | ranges.rs | Smart expansion |
| Document links | documentLink | advanced_features.rs |  |
| Signature help | signatureHelp | advanced_features.rs |  |
| Call hierarchy | callHierarchy | advanced_features.rs |  |
| Type hierarchy | typeHierarchy | type_hierarchy.rs | super/subtype navigation |
| Execute command | executeCommand | command_dispatch.rs · commands.rs | 46 `sysml.*` commands |
| File watching | didChangeWatchedFiles | workspace.rs | .sysml / .kerml + sysml.toml |
| Workspace folders | didChangeWorkspaceFolders | workspace.rs |  |

## Resolution tiers (non-blocking UI)

Tiers defined in `background.rs` (`ResolutionTier`). The UI never blocks on full analysis: cheap tiers unlock immediately, heavy work runs in the background.

```text
T1 sync< 50ms · current file▸highlighting · outline · syntax errors
T2 debounce< 200ms · same file▸go-to-def · completion
T3 bgbackground · cross-file + library▸full validation · find-refs · rename
```

## Command surface

46 `sysml.*` commands routed through `command_dispatch.rs` (45 entries) plus `sysml.cache.rebuild` (special-cased in `lib.rs`). These are `#[service_command]`-registered handlers on `sysml-service`, so the same command set is shared across CLI / LSP / REST / MCP transports.

| Category | Commands |
|---|---|
| Cache | `sysml.cache.clear` · `sysml.cache.status` · `sysml.cache.rebuild` (special) |
| Debug | `sysml.debug.status` · `sysml.debug.bundle` |
| Evaluation | `sysml.evaluate` · `sysml.evaluate.all` · `sysml.verify` · `sysml.analysis.run` |
| Simulation | `sysml.simulate.start` · `.step` · `.stop` · `.reset` |
| Orchestrate | `sysml.orchestrate.start` · `.step` · `.inject` · `.stop` |
| Sessions | `sysml.sessions.step` · `sysml.sessions.inject` |
| Scenario | `sysml.scenario.run` |
| Monte Carlo | `sysml.montecarlo.run` |
| Timeline | `sysml.timeline.getTrace` · `sysml.timeline.getSnapshot` |
| Action | `sysml.action.run` · `.start` · `.step` · `.stop` · `.reset` · `.visualize` |
| Flow viz | `sysml.flow.visualize` |
| What-if | `sysml.whatif` · `sysml.whatif.sweep` · `sysml.diagram.whatif` |
| Salsa stats | `sysml.salsa.stats` · `sysml.salsa.stats.reset` |
| Workspace | `sysml.workspace.info` · `sysml.workspace.verify` · `sysml.project.info` · `sysml.dependency.status` |
| Requirements | `sysml.requirements.trace` |
| Diagram | `sysml.diagram.open` · `.view` · `.export` · `.expand` · `.edit` |
| Model tree | `sysml.model.tree` |

## Diagram pipeline

The server is the bridge between Rust model analysis and interactive webview diagrams. Models are **server-rendered** in this process (there is no browser-side WASM engine) and pushed to the client by custom notification.

```text
analyze ModelGraph (sysml-core) → ViewModel (sysml-diagram)
  → sysml/diagram/setViewModel → webview
```

`src/diagram.rs` defines `DIAGRAM_SET_VIEW_MODEL_METHOD =
"sysml/diagram/setViewModel"`. Expand/collapse goes through the
`sysml.diagram.expand` command, which returns an updated ViewModel.

## Standard library

Library loading is asynchronous; features degrade gracefully until the stdlib is ready (check `service.readiness_for(uri).library`). The library path resolves from `SYSML_LIBRARY_PATH`, falling back to a bundled `libraries/standard` when present. The parsed library is cached under `~/.cache/sysml-rs/` to bring cold-start library availability down from seconds to sub-500ms.

## Environment variables

| Variable | Effect | Source |
|---|---|---|
| `SYSML_LIBRARY_PATH` | Override stdlib root for resolution | workspace.rs |
| `SYSML_LSP_DISABLE_INLAY_HINTS` | `=1` disables the inlay-hint capability (Zed sets this) | lib.rs |
| `SYSML_DEPENDENCY_TRACE` / `SYSML_LSP_DEPENDENCY_TRACE` | Enable dependency-resolution telemetry tracing | telemetry_control.rs |
| `SYSML_FAIL_ON_SPANLESS_DIAGNOSTICS` | Hard-fail when a diagnostic lacks a span (test/debug guard) | diagnostics.rs |
| `SYSML_REGISTRY_SYSAND_INDEX` | Point manifest features at an alternate registry index | manifest_diagnostics.rs · inlay_hints.rs |

## Dependencies

**Upstream (direct deps).**

- `sysml-service` — unified command hub (salsa-backed)

- `sysml-ide-db` — salsa `AnalysisHost` / incremental queries

- `sysml-parser-incremental` (feat `semantic`) — tree-sitter CST, sole parser

- `sysml-parser-trait` — `Parser` trait + stdlib loading

- `sysml-runtime` (feat `montecarlo`) — execution / IR / physics

- `sysml-resolve` — name resolution

- `sysml-diagram` — server-side ViewModel construction

- `sysml-project` · `sysml-manifest` — project discovery, `sysml.toml`

- `sysml-core` · `sysml-id` · `sysml-span` (serde)

- `tower-lsp` · `tokio` · `tree-sitter` · `dashmap` · `directories` · `sha2`

**Downstream.**

Top-level binary: consumed by editors (`editors/vscode`, `editors/zed`, Neovim, …) over LSP/stdio, and embedded by the REST `/lsp` WebSocket bridge via `create_service_with`. No *crate* in the workspace depends on this one.

> ⚠  **Not a dependency:** `sysml-parser-batch` (deleted) and `sysml-analysis-ir` (collapsed into `sysml-runtime`). Anything claiming otherwise is stale.

## Pitfalls & invariants

- **Salsa-first.** File content and queries go through `AnalysisHost`. Lock the `Arc<Mutex<AnalysisHost>>` briefly, take an `Analysis` snapshot, then drop the lock.

- **UX messages.** Never call `client.log_message()` directly — route through `ux_messages.rs` (see the logging contract).

- **Two highlighting systems.** Tree-sitter (`highlights.scm` in editors) and LSP semantic tokens (`semantic_tokens.rs`) are independent — check which owns a wrong color.

- **`lib.rs` is the god module** (3143 lines). It holds the `SysmlLanguageServer` struct + every `LanguageServer` handler. Feature modules compute; `lib.rs` wires them to methods.

- **URI normalization.** Use the canonical-URI / alias helpers, not raw string equality (symlinks).

Part of the [sysml-rs](../../../README.md) workspace · regenerated 2026-06-03
