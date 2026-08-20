# sysml-parser-trait

The parser-backend contract: the `Parser` trait, the `ParseResult` pipeline, and the parser-agnostic element/relationship construction toolkit every backend shares.

`Layer 2 · lang` · `parser interface` · `crate-type: rlib` · `edition 2024 · workspace`

## What this crate owns

`sysml-parser-trait` defines the boundary between *how text is parsed* and *what the rest of the workspace consumes*. A parser backend implements one trait — `Parser::parse(&[SysmlFile]) -> ParseResult` — and everything downstream (resolution, validation, the IDE database, the service layer) works against the resulting `ModelGraph` without knowing which parser produced it.

Beyond the trait, the crate carries the **shared construction toolkit**: extraction structs, an element builder, relationship-creation helpers, and standard-library loading. A backend populates plain extraction structs from its syntax nodes and lets this crate turn them into a consistent `ModelGraph` — so model shape does not drift between backends.

>  **Single real backend.** The only production implementer today is `sysml_parser_incremental::TreeSitterParser` (tree-sitter). The former Pest crate `sysml-parser-batch` has been **deleted**. `NoopParser` and `StubParser` in this crate are test-only doubles.

## Where it sits

```text
backends TreeSitterParser NoopParser StubParser
▼ implement `Parser`
this crate sysml-parser-trait ParseResult pipeline extraction + builders library loader
▼ produces `ModelGraph` + `Vec<Diagnostic>`
upstream sysml-core sysml-span
▼ consumed by
downstream sysml-parser-incremental sysml-runtime sysml-ide-db sysml-service sysml-cli · sysml-lsp-server · sysml-diagram
```

## The ParseResult pipeline

`ParseResult` wraps a `ModelGraph` plus a `Vec<Diagnostic>` and exposes a fluent, consuming pipeline. Parse produces a graph whose cross-references are still unresolved `unresolved_*` string properties; *resolution* converts those into `ElementId` references; *validation* then checks structure and relationship types.

```text
step 1 parser.parse(&files) → ParseResult (unresolved refs)
step 2 .into_resolved() / .into_resolved_with_library(lib) → ElementId refs resolved
step 3 .into_validated() → structure + relationship errors in diagnostics
```

## Public API

| Symbol | Kind | Purpose |
|---|---|---|
| `Parser` | trait | The backend contract: `parse(&[SysmlFile]) -> ParseResult`, plus `name()` and defaulted `version()`. |
| `Formatter` | trait | `format(&ModelGraph) -> String`. Declared as a planned round-trip hook — **no implementations exist yet**. |
| `SysmlFile` | struct | Parser input: `{ path, text }`, with `SysmlFile::new(path, text)`. |
| `ParseResult` | struct | `{ graph: ModelGraph, diagnostics: Vec<Diagnostic> }` + the resolution/validation pipeline. |
| `NoopParser` | struct | Test double: returns an empty graph with a single "not implemented" error. |
| `StubParser` | struct | Test double: returns an empty, error-free graph. |
| `UsageExtraction` | struct | Parser-agnostic carrier for a usage's name, typings, multiplicity, etc. |
| `DefinitionExtraction` | struct | Same, for definitions. |
| `PackageExtraction` | struct | Same, for packages. |
| `parse_multiplicity_text` | fn | Parse `[1]`/`[0..*]` → `(lower, Option<upper>)`. |
| `parse_multiplicity_full` | fn | Richer multiplicity parse → `MultiplicityResult`. |
| `library::load_standard_library` |`fn<P: Parser>` | Parse + merge the KerML/SysML/domain standard library into one `ModelGraph`. |
| `library::LibraryConfig` | struct | Where/what to load; `from_env()` reads `SYSML_LIBRARY_PATH`. |
| `library::LibraryStats` | struct | Element/package counts for a loaded library graph. |
| `library::LibraryLoadError` | enum | Error type for library loading failures. |

### ParseResult methods

#### `— *is_ok() · has_errors() · error_count()` — **

Diagnostic predicates. `is_ok()` is true when no diagnostic is an error; `has_errors()` is its inverse; `error_count()` counts error-severity diagnostics.

#### `— *into_resolved(self) -> Self` — **

Consuming. Runs `resolve_references` on the graph (turning `unresolved_*` props into `ElementId` refs) and merges resolution diagnostics in. Chains.

#### `— *resolve(&mut self) -> ResolutionResult` — **

Non-consuming variant of `into_resolved` that returns the detailed `ResolutionResult` (resolved/unresolved counts) for when you need the statistics.

#### `— *into_resolved_with_library(self, library: ModelGraph) -> Self` — **

Resolves against a pre-loaded standard library, so types like `Anything`, `Real`, `Item` resolve. Library element IDs are excluded from re-resolution to avoid double-counting failures. `resolve_with_library` is the non-consuming, statistics-returning variant.

#### `— *validate_structure() · validate_relationships() · into_validated(self)` — **

`validate_structure` checks orphans, ownership cycles, and dangling membership refs; `validate_relationships` checks relationship source/target types (best run after resolution). `into_validated` runs both and chains.

## Shared construction toolkit

A backend rarely calls the relationship helpers one-by-one; it fills extraction structs and lets the aggregate builders emit every relationship. Each helper also has a `*_with_key` variant that takes a canonical key for stable, indexed relationship IDs.

| Helper | Emits |
|---|---|
| `create_usage_relationships` | All relationships implied by a `UsageExtraction` (typings, subsettings, redefinitions, …). |
| `create_definition_relationships` | All relationships implied by a `DefinitionExtraction` (specializations, etc.). |
| `create_specialization` / `create_subclassification` | Specialization / subclassification edges. |
| `create_feature_typing` | FeatureTyping (a feature's declared type). |
| `create_subsetting` / `create_cross_subsetting` | Subsetting and cross-subsetting edges. |
| `create_redefinition` | Redefinition edge. |
| `create_reference_subsetting` | Reference subsetting edge. |
| `create_conjugated_port_typing` / `create_conjugated_port_definition` | Conjugated-port relationships. |
| `create_annotation` | Annotation relationship. |
| `build_element(kind, span)` | On each extraction struct — produces a `sysml_core::Element` of the given `ElementKind`. |

Every helper listed above also exists as `…_with_key(&mut graph, …, &key)` for deterministic relationship IDs (used by incremental reparse). That doubles the count of public `create_*` functions in `relationship_builder.rs`.

## Key modules

| Module | Responsibility | Key types / fns |
|---|---|---|
| `lib.rs` | Trait + result definitions and test doubles. | `Parser`, `Formatter`, `SysmlFile`, `ParseResult`, `NoopParser`, `StubParser` |
| `extraction.rs` | Parser-agnostic carriers + multiplicity parsing. | `UsageExtraction`, `DefinitionExtraction`, `PackageExtraction`, `parse_multiplicity_full` |
| `element_builder.rs` (private) | Turns extraction structs into `Element`s. | `impl …Extraction { build_element }` |
| `relationship_builder.rs` | Relationship-creation helpers + canonical-key variants. | `create_usage_relationships`, `create_feature_typing`, … (+ `*_with_key`) |
| `library.rs` | Standard-library discovery, parse, and merge. | `load_standard_library`, `LibraryConfig`, `LibraryStats`, `LibraryLoadError` |

## Usage

Parse, resolve against the standard library, then validate — using the real backend:

```
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_parser_trait::library::{load_standard_library, LibraryConfig};

let parser = TreeSitterParser::new();

let files = vec![SysmlFile::new(
    "model.sysml",
    "package P { part def Engine; part engine: Engine; }",
)];

// Optional: load the standard library so KerML/SysML base types resolve.
let config = LibraryConfig::from_env()?;           // reads SYSML_LIBRARY_PATH
let library = load_standard_library(&parser, &config)?;

let result = parser
    .parse(&files)
    .into_resolved_with_library(library)
    .into_validated();

if result.has_errors() {
    for diag in &result.diagnostics {
        eprintln!("{diag}");
    }
}
let graph = result.graph;  // ModelGraph ready for downstream consumers
```

Without a library (resolves only user-defined references):

```
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

let parser = TreeSitterParser::new();
let files = vec![SysmlFile::new("m.sysml", "part def Wheel;")];
let result = parser.parse(&files).into_resolved();
assert!(result.is_ok());
```

## Dependencies

**Upstream (depends on).**

- `sysml-core` — `ModelGraph`, `Element`, resolution + validation entry points.

- `sysml-span` — `Diagnostic` and severity.

- `walkdir` — recursive library-file discovery.

- `thiserror` — `LibraryLoadError`.

- `tracing` — **optional**, behind the `tracing` feature (off by default).

**Downstream (depends on this).**

- `sysml-parser-incremental` — implements `Parser` via `TreeSitterParser`; the real backend.

- `sysml-runtime`, `sysml-diagram` — consume `ParseResult`/`SysmlFile`.

- `sysml-ide-db`, `sysml-service`, `sysml-lsp-server`, `sysml-cli` — drive the parse pipeline.

- `sysml-spec-tests` — coverage harness.

- `sysml-core` — *dev-dependency only* (benchmarks); no runtime cycle.

## Invariants & pitfalls

- **One real backend.** Examples must import `sysml_parser_incremental::TreeSitterParser`. The deleted `sysml-parser-batch`/`PestParser` no longer exists.

- **`Formatter` is aspirational.** The trait is declared but has zero implementations; treat it as a planned round-trip hook.

- **Validate after resolution.** `validate_relationships` is most meaningful once `unresolved_*` props have become `ElementId` refs — chain `.into_resolved().into_validated()`.

- **Library layout.** `load_standard_library` loads `library.kernel/` first (foundational), then `library.systems/`, then `library.domain/`, all under the `LibraryConfig::library_path` base.

- **Tracing is opt-in.** `library.rs` emits spans only when built with `--features tracing`.

Part of the [sysml-rs](../../../README.md) workspace · agent guidance in `CLAUDE.md` · regenerated 2026-06-03
