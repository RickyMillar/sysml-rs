# sysml-ide-db

Salsa-backed incremental computation database for SysML v2 — the query engine between parsing and the IDE/service layer. Edit a file, only the dependent queries recompute.

`Layer 4 · tooling` · `incremental DB` · `crate-type: rlib` · `salsa 0.26` · `75 tracked queries`

## Overview

`sysml-ide-db` defines the core database types and layered query functions that power incremental recomputation for the LSP server and the unified service layer. It follows [rust-analyzer](https://github.com/rust-lang/rust-analyzer)'s architecture: source text is stored as [salsa](https://salsa-rs.github.io/salsa/) inputs, and every derived artefact — parse trees, resolved models, validation diagnostics, semantic tokens, elaborated execution seeds, physics registries, diagram graphs — is a `#[salsa::tracked]` function memoised against those inputs. When a file changes, salsa invalidates exactly the queries that transitively depend on it and serves cached results for everything else.

The invariant this crate owns is the **`AnalysisHost` (mutable owner) / `Analysis` (immutable snapshot)** split. The host applies edits on the main loop; cheap snapshots are cloned and shipped to worker threads to serve read requests concurrently. A mutation on the host cancels in-flight queries on outstanding snapshots (`salsa::Cancelled`), so callers must catch and retry.

>  **Sole parser is tree-sitter.** Parse queries drive `sysml-parser-incremental`'s `TreeSitterParser` (with the `semantic` feature) — the only parser in the workspace. There is no Pest / `sysml-parser-batch` dependency; that crate was deleted and its diagnostic enricher retired.

## Where it sits

```text
consumers sysml-lsp-server· sysml-service
▲ AnalysisHost / Analysis
L4 db sysml-ide-db
▼ tracked-query dependencies
L3 lang sysml-runtime· sysml-diagram
L1–2 lang sysml-parser-incremental· sysml-parser-trait· sysml-core
L0 lang sysml-id· sysml-span· sysml-project
```

> ⚠  **Heavyweight reach (intentional, leaky).** The execution-seed / physics / diagram query families pull in `sysml-runtime` (which depends on the `diffsol` DAE solver) and `sysml-diagram`. Any LSP build therefore transitively compiles the full simulation runtime — a deliberate tradeoff so those caches live next to the elaborated-graph queries they depend on, at the cost of build time and blast radius.

## Recompute flow

```text
mutate AnalysisHost::set_file_content()→ salsa input invalidation
▼ cancels in-flight queries (salsa::Cancelled)
snapshot AnalysisHost::analysis()→ Analysis (immutable clone)→ worker thread
▼ first access recomputes, rest served from memo
query parse → resolve → validate → elaborate
```

## Public API

The database surface: one trait, one concrete DB, four salsa inputs, and the host/snapshot pair.

| Type | Kind | Defined in | Purpose |
|---|---|---|---|
| `Db` | trait | lib.rs | Marker query trait (`: salsa::Database`); all tracked fns take `&dyn Db`. |
| `RootDatabase` | `#[salsa::db]` | lib.rs | Concrete database holding all storage; clonable (Arc bumps) for snapshots. |
| `AnalysisHost` | struct | host.rs | Mutable owner. Applies edits, loads projects, manages the standard library. |
| `Analysis` | struct | host.rs | Immutable snapshot cloned from the host; serves read requests on worker threads. |
| `SourceFile` | `#[salsa::input]` | source.rs | A single file's name + text — the root salsa input. |
| `WorkspaceConfig` | `#[salsa::input(singleton)]` | project_inputs.rs | Workspace-wide config (one per DB). |
| `SalsaProject` | `#[salsa::input]` | project_inputs.rs | Per-project metadata input. |
| `ProjectFileSet` | `#[salsa::input]` | project_inputs.rs | The set of `SourceFile`s in a project; key for workspace-wide queries. |
| `FileSet` | struct | source.rs | URI → `FileId` → `SourceFile` map (lives *outside* salsa). |
| `FileId` | newtype | source.rs | Lightweight `u32` file identifier. |
| `GlobalSymbolIndex` | struct | symbol_index_query.rs | Cross-project name → element lookup. |

## Query naming convention

Nearly every query family follows the same three-way triplet, scoped from narrowest to widest. Recognising it tells you the full surface of a module from a single function name.

**`file_X`.**

Keyed on one `SourceFile`. Single-file result, no cross-file visibility.

**`workspace_X`.**

Keyed on a `ProjectFileSet`. Merges every file in the project for cross-file resolution.

**`workspace_X_with_library`.**

As above plus the standard-library graph as a fallback lookup (the library is never merged in).

## Query families & modules

29 declared modules; 75 `#[salsa::tracked]` functions. Filter the table to find a family.

#### `Layer 0 · Source inputs` — *source.rs · project_inputs.rs*

`SourceFile`, `FileSet`, `FileId` manage file identity and text. `project_inputs.rs` adds `SalsaProject`, `WorkspaceConfig` (singleton), `ProjectFileSet`, and the `PROJECT_KIND_*` constants (`DISCOVERED`, `DISCOVERED_VIA_MANIFEST`, …) used to tag how a project was found.

#### `Layer 1 · Parse` — *parse.rs (5 queries)*

`parse_file_cst` → Send+Sync CST wrapper (outline, folding); `parse_file` → full `ModelGraph` + diagnostics (the semantic workhorse); `parse_tree` → raw `tree_sitter::Tree` wrapped in `CachedTree` for semantic tokens and keyword hover. `ParseResult` exposes `element_count()` and `has_errors()`.

#### `Layer 2 · Per-file derived` — *tokens.rs · symbols.rs · exports.rs · element_index.rs · file_source_query.rs*

Semantic tokens (`file_semantic_tokens`), document-symbol tree (`file_document_symbols`), public exports (`file_exports`), name/kind indexes (`file_name_index`, `file_kind_index` + workspace variants), position maps, and outlines.

#### `Layer 3 · Resolution` — *resolution.rs (8 queries) · ref_resolve_cache.rs*

Cross-file name resolution in three modes — single-file, with-library (fallback lookup, the ~52K-element library graph stays separate), and with-workspace (a single `cached_workspace_resolution` shared by all per-file queries to avoid O(N²) re-resolution; per-file results are filtered by element ID). `ref_resolve_cache` memoises reference-target lookups.

#### `Layer 4 · Analysis` — *analysis.rs (10 queries)*

Validation (property + semantic + structural + import-health) and elaboration (derived structure for execution). Clone-and-mutate over resolved graphs. `elaborate_file` / `elaborate_file_with_library` feed the execution-seed caches below.

#### `Execution seeds` — *eval_context.rs · eval_context_seed.rs · precompiled_constraints.rs · signal_expr_table.rs · gated_expressions.rs · port_flow_runtime.rs*

Salsa-cached inputs for the `sysml-runtime` execution engine: evaluation contexts, seed values, precompiled constraint expressions, the signal-expression table, gated/conditional expressions, and port-flow runtime data. Each follows the file / workspace / workspace-with-library triplet.

#### `Physics` — *physics.rs (6 queries)*

Physics registry and physics-health queries (`file_physics_registry`, `file_physics_health` + workspace + with-library variants) caching the physics view derived from the elaborated graph.

#### `Diagram` — *diagram.rs (3 queries)*

`file_diagram` / `workspace_diagram` / `workspace_diagram_with_library` cache server-rendered SGraph output from `sysml-diagram`, living next to the elaborated-graph queries they depend on.

#### `Views, traces & descendants` — *view_index.rs · view_filter_exprs.rs · trace_matrix_query.rs · descendants_query.rs · workspace_capabilities.rs*

View indexes and filter expressions, requirement trace-matrix queries, element descendants (keyed on `SourceFile` + `ElementId`), and workspace capability summaries.

#### `Infrastructure` — *arc_wrapper.rs · snapshot.rs · stats.rs*

`arc_wrapper.rs` defines the `salsa_arc_wrapper!` macro (identity vs fingerprint equality); `snapshot.rs` backs the `Analysis` snapshot; `stats.rs` tracks per-query execution statistics on `RootDatabase`.

## Usage

Single file — set content, snapshot, parse:

```
use sysml_ide_db::{AnalysisHost, Analysis};

let mut host = AnalysisHost::new();
let file_id = host.set_file_content("file:///test.sysml", "package Foo;".to_string());

let analysis: Analysis = host.analysis();
let sf = host.source_file(file_id).unwrap();
let parsed = analysis.parse_file(sf);
println!("elements: {}, errors: {}", parsed.element_count(), parsed.has_errors());
```

Workspace-aware resolution across two files in one project:

```
use sysml_ide_db::{AnalysisHost, ProjectFileSet, project_inputs};
use sysml_project::ProjectId;
use std::sync::Arc;

let mut host = AnalysisHost::new();
let pid = ProjectId(10);

let id1 = host.set_file_content_in_project(
    "file:///defs.sysml",
    "package Defs { part def Wheel; }".to_string(),
    pid,
);
let id2 = host.set_file_content_in_project(
    "file:///main.sysml",
    "package Main { import Defs::*; part w: Wheel; }".to_string(),
    pid,
);

let sf1 = host.source_file(id1).unwrap();
let sf2 = host.source_file(id2).unwrap();
let pfs = ProjectFileSet::new(
    host.db(),
    pid.0,
    Arc::new(vec![sf1, sf2]),
    project_inputs::PROJECT_KIND_DISCOVERED,
);
host.add_project_file_set(pfs);

let analysis = host.analysis();
let resolved = analysis.resolve_file_with_workspace(sf2, pfs);
```

## Dependencies

| Crate | Layer | Why |
|---|---|---|
| `salsa` (0.26) | ext | Incremental computation: memoisation, invalidation, cancellation. Pinned in the workspace root `Cargo.toml`. |
| `rustc-hash` | ext | Fast hash maps for file lookups. |
| `tracing` | ext | Structured logging for salsa events and query tracing. |
| `url` (2) | ext | Canonicalising `file://` URIs in `source::canonicalize_uri`. |
| `tree-sitter` | ext | Tree-sitter runtime (same version as the parser); `Tree` is Send+Sync as of 0.22.6. |
| `sysml-span` | L0 | `Diagnostic`, `Severity` in query results. |
| `sysml-id` | L0 | `ElementId`, `QualifiedName`, `ProjectId`. |
| `sysml-project` | L0 | Project / stdlib registry types for workspace management. |
| `sysml-core` | L1 | `ModelGraph`, `Element`, `Relationship` — the parsed model. |
| `sysml-parser-incremental` | L1–2 | Tree-sitter CST parsing (`semantic` feature for token extraction). The sole parser. |
| `sysml-parser-trait` | L1 | `Parser` trait / `ParseResult` contract. |
| `sysml-runtime` | L3 | Evaluation contexts & physics for the execution-seed query families. Pulls in `diffsol`. |
| `sysml-diagram` | L3 | SGraph rendering for the `diagram_query` caches. |

### Downstream

- **`sysml-lsp-server`** — primary consumer; wraps `AnalysisHost` for the LSP protocol.

- **`sysml-service`** — the unified service hub holds the host (`Arc<Mutex<AnalysisHost>>`) and dispatches commands to CLI / LSP / REST / MCP.

## Invariants & pitfalls

**Arc-wrapper equality mode.**

Query results wrap in `salsa_arc_wrapper!` with `identity` (pointer `Arc::ptr_eq`) or `fingerprint` (content hash). Use **fingerprint** when distinct Arc allocations can be semantically equal (parse / resolved / validated / elaborated graphs), else salsa over-recomputes downstream.

**Hold snapshots briefly.**

A mutation on the host blocks until every `Analysis` clone drops, cancelling their in-flight queries with `salsa::Cancelled`. Handlers must catch with `salsa::Cancelled::catch()` and drop snapshots promptly.

**Library is never merged.**

`*_with_library` queries use fallback lookup against the ~52K-element library graph; it stays separate so it isn't re-hashed into every file's graph.

**Every query result is Send+Sync + Eq+Hash.**

Required for cross-thread snapshots and salsa memoisation. New queries must satisfy `PartialEq + Eq + Hash` — wrap via the arc macro.

Part of the [sysml-rs](../../../README.md) workspace · regenerated 2026-06-03
