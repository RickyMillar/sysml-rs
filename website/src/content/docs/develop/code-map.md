---
title: Where the code lives
description: A crate-by-crate map of the sysml-rs repository — what each crate does, its key entry points, and where to start for common changes.
scope:
  - sysml-rs implementation
status: pre-alpha
last_verified_against: 023a5ef
source_of_truth:
  - crates/lang/README.md
  - crates/tooling/README.md
  - crates/testing/README.md
  - docs/developer_guide/00-architecture.md
---

A scanning map for a new contributor: what each crate and directory is for,
which files to open first, and where the deeper documentation lives. Read
[how sysml-rs works](/sysml-rs/develop/architecture/) first for the mental
model; this page assumes it.

Paths are relative to the repository root. Each crate group and most crates
carry their own `README.md` — those are the maintained deep references, and
the group READMEs
([lang](https://github.com/RickyMillar/sysml-rs/blob/main/crates/lang/README.md),
[tooling](https://github.com/RickyMillar/sysml-rs/blob/main/crates/tooling/README.md),
[testing](https://github.com/RickyMillar/sysml-rs/blob/main/crates/testing/README.md))
include full layer diagrams.

## Top level

| Directory | What it is |
|---|---|
| `crates/lang/` | The SysML v2 spec implementation: model, parser, runtime, diagrams |
| `crates/tooling/` | Service hub, salsa analysis, storage, and the four transports |
| `crates/testing/` | The full-stack conformance and regression suite |
| `editors/` | VS Code extension, Simulation App, shared expression renderer |
| `tools/` | Repo tooling: spec fetch, spec indexing, grammar iteration, dev scripts |
| `docs/developer_guide/` | The twelve deep subsystem guides |
| `references/sysmlv2/` | Pinned OMG spec sources — not committed; run `tools/fetch-references/fetch.sh` |
| `examples/` | Runnable example models the regression suite locks down |
| `website/` | This documentation portal |

## `crates/lang/` — the spec crates

Behaviour here is governed by the OMG standards, not tooling convenience.
Lang crates never depend on tooling crates.

| Crate | Purpose | Key entry points |
|---|---|---|
| `codegen` | Build-time Rust codegen from the spec TTL/XMI/Xtext sources (driven by `sysml-core/build.rs`) | `src/lib.rs`, `src/enum_generator.rs`, `src/accessor_generator.rs` |
| `sysml-id` | Stable identifiers: `ElementId`, `QualifiedName`, project/commit ids | `src/lib.rs` |
| `sysml-span` | Source locations and the `Diagnostic` type every layer reports through | `src/lib.rs` |
| `sysml-core` | The semantic model — `Element`, `Relationship`, `ModelGraph` (the universal IR), resolution, validation, elaboration, canonical JSON | `src/graph.rs`, `src/element.rs`, `build.rs` |
| `sysml-parser-trait` | The `Parser` interface and element/relationship builder abstractions | `src/lib.rs`, `src/element_builder.rs` |
| `sysml-parser-incremental` | **The sole parser.** Tree-sitter grammar plus CST → `ModelGraph` lowering | `src/lib.rs` (`TreeSitterParser`), `src/ast_builder/`, `tree-sitter/grammar.js` |
| `sysml-manifest` | `sysml.toml` / `sysml.lock` parsing and discovery | `src/manifest.rs`, `src/lock.rs` |
| `sysml-project` | Project and workspace discovery, `.kpar` archives, stdlib assets | `src/project.rs`, `src/workspace.rs`, `src/kpar/` |
| `sysml-runtime` | Execution IR + runtime: state machines, actions, flows, constraints, cases, diffsol ODE/DAE physics | `src/lib.rs`, `src/compiler/`, `src/expressions/` |
| `sysml-diagram` | View generators: `ModelGraph` → `DiagramIR` → `ViewModel`, plus PlantUML and overlay sidecars | `src/view_model.rs`, `src/ir/generators/`, `src/view_type.rs` |

Deep guides: [02-parsing](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/02-parsing.md),
[03-resolution](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/03-resolution.md),
[04-codegen](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/04-codegen.md),
[05-validation](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/05-validation.md),
[06-execution](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/06-execution.md),
[09-vis-pipeline](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/09-vis-pipeline-architecture.md).

## `crates/tooling/` — service hub and transports

| Crate | Purpose | Key entry points |
|---|---|---|
| `sysml-service` | The hub: owns all domain state; every command registered exactly once via `#[service_command]` | `src/lib.rs` (`SysmlService`), `src/command_trait.rs` (`execute_command`) |
| `sysml-service-macros` | The `#[service_command]` proc macro — generates request types, metadata, registrations | `src/lib.rs`, `src/codegen.rs` |
| `sysml-ide-db` | Salsa incremental analysis database (`AnalysisHost` / `Analysis` snapshots) | `src/host.rs`, `src/analysis.rs` |
| `sysml-query` | Transport-neutral structured query engine (filter / project / sort) | `src/lib.rs` |
| `sysml-resolve` | Transitive project-dependency resolution: path, git, kpar, registry providers | `src/resolver.rs`, `src/providers.rs` |
| `sysml-store` | Store trait with in-memory and PostgreSQL (`postgres` feature) backends; session archive | `src/lib.rs`, `src/postgres.rs` |
| `sysml-cli` | The `sysml` binary — see [CLI workflows](/sysml-rs/use/cli-workflows/) | `src/main.rs` (clap tree), one file per command family (`check.rs`, `inspect.rs`, `export.rs`, …) |
| `sysml-lsp-server` | The LSP server (tower-lsp, stdio) — see [the language server](/sysml-rs/use/lsp/) | `src/main.rs`, `src/lib.rs`, `src/command_dispatch.rs` |
| `sysml-api` | REST/WebSocket server (axum, loopback-only by default) — see [the service API](/sysml-rs/use/service-api/) | `src/main.rs`, `src/lib.rs` (routes incl. `/commands`), `src/session_ws.rs` |
| `sysml-mcp` | MCP server, one tool per service command — see [the MCP server](/sysml-rs/use/mcp/) | `src/main.rs`, `src/lib.rs` |

Deep guides: [11-sysml-service-design](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/11-sysml-service-design.md),
[07-lsp-architecture](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/07-lsp-architecture.md),
[10-mcp-server-architecture](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/10-mcp-server-architecture.md).

## `crates/testing/` — the gatekeeper

One crate, `sysml-spec-tests`: corpus coverage against the OMG spec examples,
element-kind coverage, grammar validation, spec-obligation conformance,
cross-transport parity, per-command service baselines, and performance
baselines. Coverage primitives live in `src/` (importable); the integration
suites in `tests/` drive a real in-process `SysmlService` through LSP, REST,
and direct dispatch and assert identical results. Cited spec obligations live
in `spec-obligations/`.

Start at
[`crates/testing/sysml-spec-tests/README.md`](https://github.com/RickyMillar/sysml-rs/blob/main/crates/testing/sysml-spec-tests/README.md);
run the fixture-backed gates with `cargo test -p sysml-spec-tests` (the full
corpus runs are `#[ignore]` and need the fetched references).

## `editors/` — the frontend surfaces

| Directory | Purpose | Key entry points |
|---|---|---|
| `vscode` | Thin LSP-only VS Code extension: TextMate + semantic highlighting, snippets, manifest validation — see [editors](/sysml-rs/use/editors/) | `src/extension.ts`, `src/client.ts`, `syntaxes/` |
| `simulation-app` | The Simulation App — React + TypeScript workbench over `sysml-api`, with the Tauri desktop shell in `src-tauri/` (`sysml-desktop`, a workspace member) — see [the Simulation App](/sysml-rs/use/simulation-app/) | `src/App.tsx`, `src/diagram-svg/SvgCanvas.tsx`, `src-tauri/` |
| `expression-view` | `@sysml-rs/expression-view` — shared frontend module rendering SysML expression ASTs to KaTeX | `src/ExpressionView.ts`, `src/astToKatex.ts` |

Each of `vscode` and `simulation-app` has its own README with dev-loop
instructions (the extension deploys separately from the dev build; the app
runs Vite on `:3010` proxying to `sysml-api` on `:8080`).

## `tools/` — repository tooling

| Directory | Purpose |
|---|---|
| `fetch-references` | `fetch.sh` — pinned, checksummed reconstruction of `references/sysmlv2/` from upstream OMG sources; required before the first build |
| `spec-index` | Rust tool generating the derived spec artifacts under `references/sysmlv2/derived/` — clause-anchored spec plaintexts, the Xtext rule map, and the machine-readable language pack |
| `ts-grammar` | Grammar-iteration infrastructure (build caching, measurement) that makes the ~50-minute parser regeneration loop bearable |
| `scripts` | Dev scripts: benchmarks, corpus runs, diagnostic sweeps, highlight coverage |

## Where would I change X?

**Add or improve a diagnostic.** Register the code in
`crates/lang/sysml-core/src/error_codes.rs` (codes are categorised E/R/S/V and
surface in the [diagnostics reference](/sysml-rs/reference/diagnostics/)),
then emit it from the relevant tier — structural/semantic checks in
`sysml-core` (e.g. `src/semantic_checks/`), shape-derived property
validation via the generators in `crates/lang/codegen/`. Read
[05-validation](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/05-validation.md)
first: some validators are generated from the spec shapes and must not be
hand-edited.

**Extend the grammar.** Edit
`crates/lang/sysml-parser-incremental/tree-sitter/grammar.js`, regenerate with
`tree-sitter generate --abi 14` (after `generate_from_xtext.sh`; regeneration
plus the downstream C compile costs tens of minutes — use `tools/ts-grammar/`
for iteration), then wire the new nodes through the lowering in
`crates/lang/sysml-parser-incremental/src/ast_builder/`. Corpus and grammar
gates live in `crates/testing/sysml-spec-tests`.

**Add a service command.** Add a method on `SysmlService` in
`crates/tooling/sysml-service/src/lib.rs` annotated with
`#[service_command]` — the macro registers it, and all four transports (CLI
dispatch, LSP, REST, MCP) plus the `/commands` catalogue pick it up without
per-transport wiring. Expect the service-baseline fixtures in
`crates/testing/sysml-spec-tests/fixtures/service-baseline/` and the MCP
tool-coverage gate to demand updates. Read
[11-sysml-service-design](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/11-sysml-service-design.md).

**Add a CLI flag or subcommand.** The clap tree is in
`crates/tooling/sysml-cli/src/main.rs`; each command family has its own file
(`check.rs`, `inspect.rs`, `export.rs`, `init.rs`, …). Keep logic in the
service and the CLI thin. Integration tests live in
`crates/tooling/sysml-cli/tests/` (e.g. `cli_integration.rs`), and the
[CLI reference](/sysml-rs/reference/cli-commands/) is generated from help
output rather than hand-maintained.

**Touch the runtime.** `crates/lang/sysml-runtime/src/` — lowering from
`ModelGraph` in `compiler/`, expression evaluation in `expressions/`, discrete
behaviour in `actions/` and `flows/`, constraints and cases in
`constraints.rs` and `cases/`, hybrid/continuous integration in `hybrid.rs`.
Read [06-execution](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/06-execution.md)
before changing semantics: runtime behaviour is gated by the spec-obligation
suites, and this is the youngest, fastest-moving part of the codebase.

**Change diagram rendering.** Scene *content* (nodes, edges, compartments,
which elements a view family shows) is Rust: the generators in
`crates/lang/sysml-diagram/src/ir/generators/` and the wire format in
`src/view_model.rs` — ViewModel is the only diagram contract. Visual
*presentation* (layout, shapes, interaction) is frontend:
`editors/simulation-app/src/diagram-svg/` (`SvgCanvas.tsx`, `layout.ts`,
`shapes.ts`). Follow
[09-vis-pipeline](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/09-vis-pipeline-architecture.md)
for the steps to add a view family.
