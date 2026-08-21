# sysml-rs · Developer Guide

Architecture, workflows, and conventions for the Rust implementation of the OMG SysML v2 specification. Backend (lang + tooling) only — editor surfaces are documented under `editors/*`.

`10 lang crates · 10 tooling crates · 1 testing crate` · `tree-sitter · salsa · service-hub`
## Guides

| # | Guide | Covers |
|---|---|---|
| 00 | [Architecture & Crate Layering](00-architecture.md) | Layer 0–5 rules, dependency policy, where deleted crates went |
| 01 | [Getting Started](01-getting-started.md) | Prerequisites, build, test, IDE setup, project tree |
| 02 | [Parsing](02-parsing.md) | Tree-sitter parser pipeline, grammar editing, CST → ModelGraph |
| 03 | [Resolution](03-resolution.md) | Name resolution, scope tables, standard-library integration |
| 04 | [Code Generation](04-codegen.md) | Spec-driven build-time codegen for sysml-core |
| 05 | [Validation](05-validation.md) | Property (OSLC shapes), structural, and semantic validation tiers |
| 06 | [Execution Runtime](06-execution.md) | sysml-runtime modules, execution semantics, physics |
| 07 | [LSP Architecture](07-lsp-architecture.md) | LSP server map, diagnostic sources, highlighting |
| 08 | [Logging Contract](08-logging-contract.md) | Field vocabulary, volume controls |
| 13 | [Vis Pipeline Architecture](13-vis-pipeline-architecture.md) | ModelGraph → generators → DiagramIR → ViewModel → React-SVG renderer |
| 19 | [MCP Server Architecture](19-mcp-server-architecture.md) | sysml-mcp transport, one tool per service command |
| 20 | [sysml-service Design](20-sysml-service-design.md) | #[service_command] dispatch, transport adapters, state model |


## Architecture at a glance

Lower layers never depend on higher ones. CLI / LSP / REST / MCP are thin transports that all route through `sysml-service`; the service owns the model (a salsa `AnalysisHost`) and the runtime sessions. Tree-sitter is the sole parser.

```text
Transports sysml-clisysml-lsp-serversysml-apisysml-mcp
▼ all dispatch through
Service sysml-servicesysml-service-macros
▼
Tooling sysml-ide-db (salsa)sysml-resolvesysml-querysysml-store
▼
Lang feat. sysml-runtime (exec + IR + physics)sysml-diagram (rlib, server-rendered)
▼
Frontend sysml-parser-incremental (tree-sitter)impl sysml-parser-trait::Parser
▼ produces
Core sysml-core — Element · Relationship · ModelGraph
▼
Foundation sysml-idsysml-spansysml-projectsysml-manifestsysml-codegen
```

Full rules and the crate-consolidation table live in [00-architecture.md](00-architecture.md).

## Crate consolidation (for readers coming from older code)

Two crate-graph surgeries shipped recently. If a stale doc or `use` statement names one of these, here is where it went:

| Old crate | Now | Note |
|---|---|---|
| `sysml-parser-batch` | deleted | Pest PEG parser removed; tree-sitter (`sysml-parser-incremental`) is the sole parser and implements `Parser`. |
| `sysml-analysis-ir` | `sysml-runtime` | Full analysis IR + execution + physics now in one crate. |
| `sysml-meta` | `sysml-core` | Metadata / applicability folded in. |
| `sysml-canon` | `sysml-core` | Canonical JSON is `sysml-core::json`. |
| `sysml-store-postgres` | `sysml-store` | PostgreSQL is a backend behind the `postgres` feature. |

## Current status

**Parsing.**

Single tree-sitter parser (`sysml-parser-incremental`), incremental and error-tolerant. Corpus coverage tracked in `sysml-spec-tests`.

**Resolution.**

Multi-file workspace resolution with standard-library merge, salsa-cached in `sysml-ide-db`.

**Execution.**

`sysml-runtime`: expressions, state machines, actions, flows, constraints, cases, and a `diffsol`-backed physics solver.

**Service surface.**

171 `#[service_command]` methods, exposed identically over CLI / LSP / REST / MCP (172 MCP tools — one per command, plus `sysml_command_catalog`; 100% command coverage enforced by CI). The count moves whenever a command is added; `command_count()` is the authority, and [00-architecture.md](00-architecture.md) shows how to read it.

**LSP.**

Diagnostics, completion, hover, navigation, rename, semantic tokens, code actions, inlay hints — all via `SysmlService`.

**Diagrams.**

Server-rendered and delivered to editors — no in-browser parse/render engine. `sysml-diagram` emits two formats: `ViewModel` (renderer-agnostic scene + design tokens, the going-forward wire format) and the legacy Sprotty-compatible `SModel` JSON that CLI/LSP/REST/MCP still ship until they migrate.

## Key design principles

- **Spec compliance.** Behavior matches the OMG SysML v2 specification; `ElementKind` (182 variants) and property accessors are generated from the spec TTL.

- **Layered architecture.** Clear lang / tooling separation; dependencies only point downward (see [00-architecture.md](00-architecture.md)).

- **Incremental processing.** Tree-sitter + salsa give sub-millisecond reparse and cached re-analysis.

- **One service, many transports.** CLI / LSP / REST / MCP are thin; the service owns the model so behavior cannot drift between transports.

## Error-handling conventions

| Pattern | Crates | When to use |
|---|---|---|
| `thiserror` (structured) | `sysml-core`, `sysml-runtime`, `sysml-codegen` | Library crates exposing typed error enums to callers. |
| `anyhow` (dynamic) | `sysml-cli`, `sysml-api` (binaries) | Application/binary crates where errors are reported to users. |
| `Vec<Diagnostic>` | `sysml-runtime` health modules, `sysml-ide-db` validation | Validation / analysis passes that collect multiple issues. |

- New library crates: `thiserror` with a crate-level `Error` enum.

- Validation / health functions: return `Vec<sysml_span::Diagnostic>`.

- The LSP server converts every error type to an LSP `Diagnostic` for publishing.

- Do not mix patterns within a single crate.

## Contributing

- Read the relevant guide for your area.

- Research the SysML v2 spec before implementing (see `references/sysmlv2/`).

- Run `cargo test`; for parser/diagnostic work run the corpus and `tools/scripts/diagnostic_sweep.sh`.

Part of the [sysml-rs](../../README.md) workspace
