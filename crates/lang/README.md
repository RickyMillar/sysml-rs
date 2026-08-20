# lang/

The ten crates that implement the SysML v2 specification itself — identifiers, diagnostics, the semantic model, the tree-sitter parser, manifests, project discovery, the execution runtime, and diagram exporters. Spec-defined behavior, not tooling convenience.

`crate group · lang` · `10 crates` · `4 layers` · `downward-only deps`

These are **spec crates**: their behavior is governed by OMG standards (KerML + SysML v2 vocabularies, shapes, and the Systems Modeling API), not by what is convenient for tooling. The sibling [`tooling/`](../tooling/README.md) group builds the LSP server, CLI, store, REST/MCP services, and layout engine *on top of* these. Always research the spec before changing lang behavior — see `references/sysmlv2/REFERENCE_MAPPING.md`.

## Layer & dependency graph

Each layer depends only **downward**. `codegen` runs at build time (via `sysml-core/build.rs`) to emit the spec-derived type system. Layer 0 compiles first, then Layer 1, then Layer 2 (parallel), then Layer 3 (parallel).

```text
Layer 3 sysml-runtime sysml-diagram ↳ depends on sysml-runtime (diffsol) → rlib-only, no wasm
▲ features consume the model
Layer 2 sysml-parser-trait sysml-parser-incremental sysml-manifest sysml-project
▲ the parser produces ModelGraph; project/manifest locate & configure sources
Layer 1 sysml-core Element · Relationship · ModelGraph (the universal IR)
▲ everything operates on ModelGraph
Layer 0 codegen sysml-id sysml-span
```

>  **ModelGraph is the universal IR.** The tree-sitter parser (`sysml-parser-incremental`) lowers source into a `ModelGraph` from `sysml-core`; `sysml-runtime` lowers that into its own execution IR before solving with diffsol; `sysml-diagram` renders directly from `ModelGraph`. Parser → core → consumers, never sideways.

## Crates

Sort by any column; filter to jump to a crate. Each name links to its own README.

| Crate | Layer | Role | Key types | crate-type |
|---|---|---|---|---|
| [`sysml-codegen`](codegen/README.md) | 0 | Build-time Rust codegen from TTL / XMI / Xtext specs | TTL vocab parser, ElementKind emitter, validator generators | rlib + bins |
| [`sysml-id`](sysml-id/README.md) | 0 | Stable identifiers across the model | ElementId, QualifiedName, ProjectId, CommitId | rlib |
| [`sysml-span`](sysml-span/README.md) | 0 | Source locations & diagnostics | Span, Diagnostic, Severity | rlib |
| [`sysml-core`](sysml-core/README.md) | 1 | The semantic model — the universal IR | Element, Relationship, ModelGraph; resolution, validation, JSON | rlib |
| [`sysml-parser-trait`](sysml-parser-trait/README.md) | 2 | Parser interface abstraction | Parser trait, ParseResult | rlib |
| [`sysml-parser-incremental`](sysml-parser-incremental/README.md) | 2 | **Sole parser.** Tree-sitter CST → ModelGraph (IDE-focused, fast) | TreeSitterParser (impls Parser trait), CST → ModelGraph lowering | rlib |
| [`sysml-manifest`](sysml-manifest/README.md) | 2 | Manifest & lock-file parsing | `sysml.toml` / `sysml.lock` models | rlib |
| [`sysml-project`](sysml-project/README.md) | 2 | Project discovery, workspace info, .kpar archives | Project, workspace discovery (consumes sysml-manifest) | rlib |
| [`sysml-runtime`](sysml-runtime/README.md) | 3 | IR + execution + physics (absorbed former `sysml-analysis-ir`) | state machines, actions, flows, constraints, diffsol DAE solve | rlib |
| [`sysml-diagram`](sysml-diagram/README.md) | 3 | Diagram IR + Sprotty SModel / PlantUML exporters | DiagramIR, ViewGenerator, SGraph; server-rendered | rlib (no cdylib) |

## Cross-cutting patterns

**Codegen pipeline.**

TTL / XMI / Xtext spec files are parsed by `sysml-codegen` at build time (driven from `sysml-core/build.rs`), producing the `ElementKind` enum, value enums, typed property accessors, and validation dispatchers.

> ⚠ **Never hand-edit `.generated.rs` files.** Coverage validators panic if the spec sources and generated code diverge.

**Spec-driven types.**

All element types come from `Kerml-Vocab.ttl` + `SysML-vocab.ttl`; all property constraints from `*-shapes.ttl`; relationship constraints from XMI. Behavior is defined by the OMG standard, not invented.

**Error handling.**

`thiserror` for library error types. Validation functions return `Vec<Diagnostic>` (from `sysml-span`). The parser returns a `ParseResult` carrying both the `ModelGraph` and diagnostics.

**Feature gates.**

`serde` is feature-gated in the foundation crates (`sysml-id`, `sysml-span`, `sysml-core`). Use `#[cfg(feature = "serde")]`; never add an unconditional serde dependency at Layers 0–1.

## Recent crate-graph surgery

>  **Tree-sitter is the sole parser.** The former Pest crate `sysml-parser-batch` was *deleted* under ADR-014 Q1 (flag-then-delete complete). `TreeSitterParser` implements `sysml_parser_trait::Parser` and returns a real `ModelGraph`-backed `ParseResult`.

>  **analysis-IR collapsed into the runtime.** The former `sysml-analysis-ir` crate was deleted and its IR + execution + physics layer folded into `sysml-runtime`, which now owns the full pipeline (and pulls in diffsol). Because of that diffsol dependency, `sysml-diagram` is rlib-only and cannot target wasm32 — diagrams are server-rendered (via the LSP server) rather than generated in a browser.

## Build & test

```
# Build the spec type system (runs codegen, regenerates sysml-core)
cargo build -p sysml-core

# Test the lang crates
cargo test -p sysml-id -p sysml-span -p sysml-core \
           -p sysml-parser-incremental -p sysml-runtime -p sysml-diagram

# Corpus coverage (parser vs spec examples)
SYSML_CORPUS_PATH=references/sysmlv2 \
  cargo test -p sysml-spec-tests corpus_coverage -- --ignored
```

Integration tests that need parsing use `sysml-parser-incremental` with the `semantic` feature (which enables `ModelGraph` construction). Each crate keeps its own `#[cfg(test)] mod tests` colocated with source.

Part of the [sysml-rs](../../README.md) workspace · agent guidance in `CLAUDE.md` · regenerated 2026-06-03
