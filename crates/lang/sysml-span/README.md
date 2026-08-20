# sysml-span

Source locations, diagnostics, and severity levels — the shared vocabulary every layer above uses to point at code and report problems.

`Layer 0 · lang` · `foundation · spans + diagnostics` · `crate-type: rlib` · `zero workspace deps`

## Overview

`sysml-span` is the bottom of the dependency graph. It owns three primitives that everything else in the toolchain passes around: a **Span** (where in a file something is), a **Severity** (how bad it is), and a **Diagnostic** (a structured message attached to a span). Every analysis pass — parse, structural check, name resolution, semantic validation, constraint evaluation — produces `Vec<Diagnostic>`, and every transport (CLI, LSP, REST, MCP) consumes them.

The crate has **no workspace dependencies** and compiles to pure `std` when its optional features are off. This is deliberate: a Layer-0 leaf must be cheap to depend on and impossible to cycle through.

¹ optional, behind feature flags. With all features off, the only dependency is the standard library.

```text
consumers sysml-core sysml-parser-trait sysml-parser-incremental sysml-runtime sysml-diagram
sysml-query sysml-ide-db sysml-lsp-server sysml-service sysml-cli
▲ depend on
Layer 0 sysml-span → std serde¹ schemars¹ annotate-snippets¹
```

## Public API

| Type | Kind | Purpose |
|---|---|---|
| `Span` | struct | Byte range `start..end` in a file, plus optional 1-indexed `line`/`col`. `Display` emits `file:line:col`. |
| `Severity` | enum | `Info < Warning < Error` (ordered). Default is `Error`. |
| `Diagnostic` | struct | Builder-pattern message: severity, optional code/span/notes/related/tags/tier. |
| `Diagnostics` | struct | `Vec<Diagnostic>` wrapper with `has_errors()`, `error_count()`, iterator + `Extend`/`FromIterator`. |
| `RelatedLocation` | struct | Secondary span + message attached to a diagnostic ("first defined here"). |
| `DiagnosticTag` | enum | `Unnecessary` / `Deprecated` — maps to LSP diagnostic tags for IDE rendering. |
| `DiagnosticTier` | enum | Which analysis context a diagnostic depended on (8 variants). Drives the readiness publish filter. |
| `LineIndex` | struct | Pre-computed line-offset table; O(log n) byte-offset → (line, col). |
| `SourceProvider` | trait | *(feature `pretty`)* Fetch source text by filename for snippet rendering. |
| `HashMapSourceProvider` | struct | *(feature `pretty`)* In-memory `SourceProvider` backed by a map. |
| `DiagnosticRenderer` | struct | *(feature `pretty`)* Renders diagnostics with source snippets via `annotate-snippets`. |

#### `Span — locations in source`

Constructors: `new(file, start, end)`, `with_location(file, start, end, line, col)`, `point(file, offset)`, `synthetic()`. Queries: `len()`, `is_empty()`, `contains(offset)`, `merge(&other)`. Derives `Ord`/`Hash`, so spans key maps and sort deterministically.

```
use sysml_span::Span;

let a = Span::new("file.sysml", 10, 20);
assert_eq!(a.len(), 10);
assert!(a.contains(15));

let b = Span::with_location("file.sysml", 30, 40, 5, 3);
assert_eq!(b.line, Some(5));

// merge spans the union of both ranges
let m = a.merge(&b);
assert_eq!((m.start, m.end), (10, 40));
```

#### `Diagnostic — structured messages (builder)`

Start with `error`/`warning`/`info`, then chain builder methods. Public fields: `severity`, `code`, `message`, `span`, `notes`, `related`, `tags`, `tier`.

```
use sysml_span::{Diagnostic, DiagnosticTier, Span};

let d = Diagnostic::error("unexpected token")
    .with_code("E001")
    .with_span(Span::with_location("file.sysml", 10, 20, 5, 3))
    .with_note("expected ';' here")
    .with_related(Span::new("file.sysml", 0, 7), "block opened here")
    .with_tier(DiagnosticTier::Parse);

assert!(d.is_error());
assert_eq!(d.code.as_deref(), Some("E001"));
```

Builder methods: `with_span`, `with_code`, `with_note`, `with_notes`, `with_related`, `with_related_location`, `with_tag`, `with_tier`. `Ord` sorts by span, then severity, then message — stable for snapshot tests.

#### `DiagnosticTier — readiness classification`

Tags *what context a diagnostic needed* so the readiness filter can decide whether it is safe to publish given the current `AnalysisHost` state. A diagnostic that depended on context not yet loaded (e.g. cross-file name resolution before the workspace is indexed) is filtered out rather than emitted-then-silenced. Ordered cheapest → most-dependent; defaults to `Parse` so pre-tier callers and pre-existing JSON snapshots remain backward-compatible.

| Variant | Context required |
|---|---|
| `Parse` *(default)* | The file's own text. Always safe to emit. |
| `StructuralLocal` | Within-file structural checks. Parse only. |
| `NameResLocal` | Bare-name resolution inside the file's own scope. |
| `NameResLibrary` | Resolution needing the standard library loaded. |
| `NameResWorkspace` | Resolution needing the project file set indexed. |
| `ImportHealth` | Import-health diagnostics. |
| `Semantic` | Semantic passes that run after resolve. |
| `Constraint` | Diagnostics requiring evaluated runtime values. |

>  **Maintenance note.** The variant doc-comments in `src/lib.rs` reference concrete code families (e.g. `S001-S004`, `IM010-IM012`, `PH*/SM*/V*`) owned by higher layers. Those examples can drift as upstream code taxonomies evolve — treat them as illustrative, not authoritative.

#### `LineIndex — fast offset → line/col`

Build once per source, then look up positions in O(log n) via `partition_point`. Line and column are 1-indexed.

```
use sysml_span::LineIndex;

let idx = LineIndex::new("package P;\npart def E;\n");
let (line, col) = idx.line_col(11); // start of second line
assert_eq!((line, col), (2, 1));
assert_eq!(idx.line_count(), 3);
```

#### `pretty module — snippet rendering *(feature pretty, default)*`

`DiagnosticRenderer` turns a `Diagnostic` into a terminal-style snippet using `annotate-snippets`. `SourceProvider` abstracts source lookup; `HashMapSourceProvider` is the in-memory implementation. `Diagnostic::render_snippet(&provider)` is the convenience shortcut used by the `examples/render_diagnostics.rs` example.

```
use sysml_span::{Diagnostic, HashMapSourceProvider, Span};

let source = "package Demo {\n  part engine: Engine;\n}\n";
let diag = Diagnostic::error("undefined type")
    .with_code("E010")
    .with_span(Span::with_location("demo.sysml", 33, 39, 2, 16))
    .with_note("declare 'Engine' or import it");

let mut provider = HashMapSourceProvider::new();
provider.insert("demo.sysml", source);

let rendered = diag.render_snippet(&provider);
println!("{rendered}");
```

## Features

| Feature | Default | Pulls in | Unlocks |
|---|---|---|---|
| `pretty` | yes | `annotate-snippets` | `DiagnosticRenderer`, `SourceProvider`, `HashMapSourceProvider`, `Diagnostic::render_snippet` |
| `serde` | no | `serde` | `Serialize`/`Deserialize` on all public types |
| `schemars` | no | `schemars` | `JsonSchema` derive on `Span`, `Severity`, `Diagnostic`, `DiagnosticTag`, `DiagnosticTier`, `RelatedLocation` |

With every feature disabled the crate is pure `std` — no external dependencies at all.

## Invariants & pitfalls

- **Builder, not constructor.** Diagnostics are always built fluently: `Diagnostic::error(msg).with_span(..).with_code(..)`. There is no positional constructor.

- **Deterministic ordering.** `Diagnostic`'s `Ord` sorts by span, then severity, then message — so snapshot tests are stable regardless of discovery order.

- **1-indexed lines/cols, 0-indexed offsets.** `Span.start`/`end` are byte offsets; `line`/`col` and `LineIndex::line_col` are 1-indexed.

- **Tier defaults to `Parse`.** Emission sites left untagged behave exactly as before the tier model existed; only callers that opt in via `with_tier` participate in the readiness filter.

- **Layer-0 leaf.** This crate must stay free of workspace dependencies. Keep higher-layer concepts (element kinds, resolution state, runtime values) out of it.

## Build & test

```
cargo test  -p sysml-span            # unit tests
cargo test  -p sysml-span --doc      # doctests on every public type
cargo bench -p sysml-span            # span_benchmarks (criterion)
cargo run   -p sysml-span --example render_diagnostics
```

Part of the [sysml-rs](../../../README.md) workspace · regenerated 2026-06-03
