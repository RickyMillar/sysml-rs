# Name Resolution

This guide covers how sysml-rs binds name references to their targets. After parsing produces a `ModelGraph` whose cross-references live as `unresolved_*` string properties, resolution rewrites them into resolved `ElementId`s.

For where resolution sits in the layering, see [00-architecture.md](00-architecture.md). For what produces resolution's input, see [02-parsing.md](02-parsing.md). For what consumes its output, see [05-validation.md](05-validation.md).

## Two callers, one algorithm

The actual resolution algorithm — scope walking, import processing, library lookup — lives in `sysml-core` (`sysml_core::resolution`). It is called from two places:

```
                ┌─────────────────────────────────────┐
                │  sysml_core::resolution::           │
                │  resolve_references{,_excluding}    │
                └─────────────┬───────────────────────┘
                              │ shared algorithm
              ┌───────────────┴────────────────┐
              ▼                                 ▼
  ParseResult::into_resolved        sysml-ide-db salsa queries
  ParseResult::resolve_with_library resolve_file{,_with_library,
                                    _with_workspace,
                                    _with_workspace_and_library}
              │                                 │
              ▼                                 ▼
        CLI, runtime,                    LSP, MCP, REST
        library loading                  (via SysmlService)
```

Both callers operate on the same `ModelGraph` shape; ide-db adds **salsa caching** and **workspace merging** on top.

## Pipeline

Resolution is a single pass over the graph, but conceptually three phases:

### Phase 1 — scope table construction

Every namespace element (Package, Definition, Usage with members) gets a scope:

```rust
struct ScopeTable {
    members: HashMap<String, ElementId>,  // direct children
    imports: HashMap<String, ElementId>,  // populated in phase 2
}
```

### Phase 2 — import resolution

Import statements populate imported names into the enclosing scope:

| Form | Effect |
|------|--------|
| `import Base::Element;` | Bind `Element` in this scope |
| `import Base::*;` | Bind every public member of `Base` |
| `import Base::**;` | Recursive: bind every package member transitively |

### Phase 3 — reference resolution

For each `unresolved_*` property on each element:

1. Look up the name in the current scope's `members`.
2. If not found, check the current scope's `imports`.
3. If still not found, walk up to the parent scope and repeat.
4. If still not found, consult the merged library index.
5. If still not found, emit an "Unresolved reference" diagnostic and leave the original `unresolved_*` string in place.

## Reference shapes

| Reference form | Strategy |
|----------------|----------|
| Simple name `Vehicle` | Scope walk from enclosing scope, then library |
| Qualified name `Parts::Engine` | Resolve `Parts` by simple-name rules, then look up `Engine` in `Parts`'s scope |
| Feature chain `car.engine.power` | Resolve `car`, then `engine` in its type's features, then `power` in *that* type's features |

Feature chains require types to already be resolved, which is why resolution is a single pass that processes typing relationships (`:`, `:>`) before feature-access expressions.

## Standard library

The standard library provides root types like `Integer`, `Real`, `String`, plus ISQ/SI quantity hierarchies. It is parsed once (via the tree-sitter `Parser` — `load_standard_library<P: Parser>` is generic over any parser, and tree-sitter is the only one) and reused across resolution calls.

### Loading the library (parser-trait layer)

```rust
use sysml_parser_trait::library::{load_standard_library, LibraryConfig};
let config = LibraryConfig::from_env()?;  // honors SYSML_LIBRARY_PATH
let library = load_standard_library(&parser, &config)?;
```

### Loading in ide-db (salsa-cached)

The LSP/MCP path loads the library once into a salsa input and caches it:

```rust
let lib_data: LibraryData = compute_stdlib_library_data(&db, corpus_path);
let lib_graph: LibraryGraph = /* salsa-cached ModelGraph */;
```

### Resolution with library

Both layers offer a "with library" variant. The library is merged into the user's graph and then resolution runs **excluding** library elements (they were already resolved during library load — re-resolving would double-count failures).

```rust
// Parser-trait layer (CLI / runtime / library loading)
let result = parser.parse(&files).into_resolved_with_library(library);

// ide-db layer (LSP / MCP)
let resolved: ResolvedModel = analysis.resolve_file_with_workspace_and_library(
    source_file, project_files, library,
);
```

### Authoring policy

Examples and fixtures in this repo follow **Policy A**: explicitly import every standard-library root used by the file.

```sysml
package Example {
    private import ScalarValues::*;
    private import ISQ::*;
    private import SI::*;

    part def Motor {
        attribute rpm : Integer;
        attribute torque :> ISQ::torque = 210 [SI::N * SI::m];
    }
}
```

Rationale: explicit imports give predictable diagnostics, completion, and cross-file behavior across editors. Implicit/transitive visibility is allowed by the spec but not relied on in our authoring examples.

## ide-db resolution surface

`sysml_ide_db::Analysis` exposes four resolution entry points, in order of increasing context:

| Method | Inputs | Use case |
|--------|--------|----------|
| `resolve_file(sf)` | Single source file | Quick single-file resolution (no library, no project) |
| `resolve_file_with_library(sf, lib)` | + library | Single file against stdlib |
| `resolve_file_with_workspace(sf, project_files)` | + sibling files | Multi-file project, no stdlib |
| `resolve_file_with_workspace_and_library(sf, project_files, lib)` | All of the above | Production LSP path |

All four are salsa-tracked queries — re-invoking with the same inputs returns cached results. The workspace variants use `cached_workspace_resolution_with_library` (`crates/tooling/sysml-ide-db/src/resolution.rs:359`) to share the merged workspace graph across files in the same project, so opening 50 files in a project doesn't re-merge the graph 50 times.

> **Drift note**: the workspace-merging recipe currently has two implementations (parser-trait's `into_resolved_with_library` and ide-db's `cached_workspace_resolution_with_library`). X5 (resolution unification) is the planned consolidation; until it lands, both paths must stay behaviorally equivalent.

## Import-health diagnostics

Beyond unresolved-reference errors, `sysml-core` emits **import-health** diagnostics flagging suspicious imports (importing nothing, importing into a non-namespace, etc.). Three entry points:

| Function | Context | Caller |
|----------|---------|--------|
| `import_health_diagnostics(graph)` | No library, no workspace | CLI single-file checks |
| `import_health_diagnostics_with_library(graph, library)` | Library context only | Single-file with stdlib |
| `import_health_diagnostics_with_context(graph, library, workspace)` | Full context | LSP, sysml-service |

The `_with_context` variant lets a file resolve imports through sibling files in the same workspace without false positives. This is the canonical path for LSP and SysmlService (X6).

## Diagnostics shape

Resolution diagnostics are standard `sysml_span::Diagnostic`s with severity, span, message, and an optional structured code:

```rust
for diag in resolution.diagnostics.iter() {
    if diag.is_error() {
        eprintln!("{}", diag);
        // "Unresolved reference 'Foo' at Vehicle.sysml:42:5"
    }
}
```

Stats are returned separately by `resolve` and `resolve_with_library`:

```rust
let stats = result.resolve_with_library(library);
println!("Resolved: {}, Unresolved: {}", stats.resolved_count, stats.unresolved_count);
```

Current corpus resolution rate: **~98.9%**.

## Debugging resolution issues

### Inspect a single file

```bash
sysml inspect path/to/file.sysml --diagnostics
```

Shows resolution diagnostics for that file. Add `--json` for machine-readable output.

### Run the corpus smoke test

```bash
SYSML_CORPUS_PATH=references/sysmlv2 \
  cargo test -p sysml-spec-tests corpus_smoke_test -- --ignored --nocapture
```

### Multi-file corpus resolution

```bash
SYSML_CORPUS_PATH=references/sysmlv2 \
  cargo test -p sysml-spec-tests corpus_resolution_multi_file -- --ignored --nocapture
```

### Common causes

| Symptom | Likely cause |
|---------|--------------|
| `Unresolved reference 'Integer'` in a stdlib type | Library not loaded / `SYSML_LIBRARY_PATH` not set |
| `Unresolved reference` for cross-file element in LSP | Resolution running without workspace context — use `resolve_file_with_workspace_*` |
| Reference resolves in CLI but not LSP | Salsa stale; check whether `set_file_content_in_project` was called for the dependency |
| Library type "found but wrong" | Library index built before parse, or library merge missed `rebuild_indexes` |

## Key files

| Concern | File |
|---------|------|
| Resolution algorithm | `crates/lang/sysml-core/src/resolution/` |
| `ParseResult::into_resolved{_with_library}` | `crates/lang/sysml-parser-trait/src/lib.rs:138-220` |
| Library loading (parser-trait) | `crates/lang/sysml-parser-trait/src/library.rs` |
| Salsa-cached resolution | `crates/tooling/sysml-ide-db/src/resolution.rs` |
| Library data salsa input | `crates/tooling/sysml-ide-db/src/resolution.rs` (`LibraryData`, `LibraryGraph`); installed via `AnalysisHost::set_library` in `host.rs` |
| Import-health diagnostics | `crates/lang/sysml-core/src/import_health.rs` |
| ResolvedModel container | `crates/tooling/sysml-ide-db/src/resolution.rs` |

## Performance notes

- Library and workspace indexes are hash maps — O(1) name lookup.
- Salsa caches the resolved graph by `(SourceFile, ProjectFileSet, LibraryGraph)` triple. Editing one file in a project invalidates only that file's resolved model; sibling files keep their cached resolution.
- Library element IDs are collected once and passed to `resolve_references_excluding` so library re-resolution failures are not double-counted.

## Related documentation

- [00-architecture.md](00-architecture.md) — layering overview.
- [02-parsing.md](02-parsing.md) — produces the unresolved input.
- [05-validation.md](05-validation.md) — runs after resolution.
- [07-lsp-architecture.md](07-lsp-architecture.md) — how the LSP consumes resolved models.
- `crates/lang/sysml-core/README.md` — semantic model details.
- `crates/tooling/sysml-ide-db/README.md` — salsa query layer.
