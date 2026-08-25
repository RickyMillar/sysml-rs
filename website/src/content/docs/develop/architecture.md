---
title: How sysml-rs works
description: The mental model of the sysml-rs architecture — one parsing pipeline, one semantic graph, one service hub behind four transports — and where each subsystem's deep documentation lives.
scope:
  - sysml-rs implementation
status: pre-alpha
last_verified_against: 023a5ef
source_of_truth:
  - docs/developer_guide/00-architecture.md
  - crates/lang/README.md
  - crates/tooling/README.md
  - crates/tooling/sysml-service/src/command_trait.rs
known_limitations: /sysml-rs/reference/known-limitations/
---

This page gives you the mental model of sysml-rs before you read any code: what
the pipeline is, which abstractions everything else hangs off, and where each
subsystem's deep documentation lives. The companion
[code map](/sysml-rs/develop/code-map/) tells you which crate and file to open.

The deep, canonical material is the
[developer guide](https://github.com/RickyMillar/sysml-rs/tree/main/docs/developer_guide)
in the repository — twelve numbered guides covering each subsystem in detail.
This page orients; the guides explain.

## The pipeline

Text goes in and a queryable, executable model comes out:

```text
 .sysml / .kerml source
        │
        ▼
 tree-sitter CST          sysml-parser-incremental (the sole parser)
        │  lowering (ast_builder)
        ▼
 ModelGraph               sysml-core — the ONE intermediate representation
        │
        ▼
 name resolution +        sysml-core resolution & validation tiers,
 validation               cached incrementally by sysml-ide-db (salsa)
        │
        ▼
 consumers                execution (sysml-runtime) · queries (sysml-query)
                          · diagrams (sysml-diagram) · every transport
```

Two facts carry most of the architecture:

1. **`ModelGraph` is the universal IR.** Defined in
   `crates/lang/sysml-core/src/graph.rs`, it is what the parser produces and
   what the runtime, query engine, diagram generators, and all four transports
   read. Nothing bypasses it and nothing talks sideways: parser → core →
   consumers.
2. **One service hub fronts everything.** `SysmlService`
   (`crates/tooling/sysml-service/src/lib.rs`) owns the model state and the
   runtime sessions; CLI, LSP, REST, and MCP are thin transports that dispatch
   named commands into it. The same operation behaves identically no matter
   how you reach it.

## Parsing: tree-sitter, and only tree-sitter

`sysml-parser-incremental` wraps a generated tree-sitter grammar
(`crates/lang/sysml-parser-incremental/tree-sitter/grammar.js`) whose keyword,
operator, and enum tables are derived from the OMG Xtext grammars by
`generate_from_xtext.sh`. The parser is incremental and error-tolerant:
unsupported syntax surfaces as a parse diagnostic rather than silently doing
something else. The CST is lowered into a `ModelGraph` by the `ast_builder`
modules.

There is no second parser. An earlier Pest-based batch parser was deleted; any
document that mentions it is stale.

Grammar changes are expensive: regenerating `parser.c` (ABI 14, pinned — ABI 15
compiles and then segfaults) takes tens of minutes. `tools/ts-grammar/` exists
to make that iteration loop bearable.

Deep guide: [02-parsing.md](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/02-parsing.md)

## The semantic graph and the spec-is-gospel posture

`sysml-core` holds the semantic model: `Element`, `Relationship`, and
`ModelGraph`, plus resolution, validation, elaboration, and canonical JSON.
Its type system is not hand-written — `crates/lang/sysml-core/build.rs` runs
the `sysml-codegen` generators over the fetched OMG spec sources (TTL
vocabularies, shapes, XMI, Xtext) to emit the `ElementKind` enum, typed
property accessors, and validation dispatchers. That is why a fresh clone must
run `tools/fetch-references/fetch.sh` before `cargo build`, and why
`.generated.rs` files are never hand-edited.

This is the project's core posture: **the specification is the authority**.
Where implementation and spec disagree, the implementation is wrong; language
behaviour changes need a spec citation, not an argument from taste. The twin
rule is **fail hard rather than degrade quietly** — an unresolved name or
unsupported construct produces a precise diagnostic, never a plausible
fallback. Both rules are spelled out in
[CONTRIBUTING.md](https://github.com/RickyMillar/sysml-rs/blob/main/CONTRIBUTING.md).

Name resolution runs over the whole workspace plus the standard library;
validation is tiered (structural, property/shape-derived, semantic), and every
diagnostic carries a stable code from the registry in
`crates/lang/sysml-core/src/error_codes.rs` — the same codes documented in the
[diagnostics reference](/sysml-rs/reference/diagnostics/).

Deep guides:
[03-resolution.md](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/03-resolution.md) ·
[04-codegen.md](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/04-codegen.md) ·
[05-validation.md](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/05-validation.md)

## The service hub: one command registry, four transports

Every user-visible operation is a method on `SysmlService` annotated with
`#[service_command]` (a proc macro from `sysml-service-macros`). The macro
generates the typed request, the metadata, and a link-time registration, so
the registry assembles itself — there is no hand-maintained command list.
Dispatch is one function: `execute_command` in
`crates/tooling/sysml-service/src/command_trait.rs` looks a command up by name
and invokes it against the shared service.

The four transports are deliberately thin:

- **CLI** (`sysml-cli`) — see [CLI workflows](/sysml-rs/use/cli-workflows/)
  and the [command reference](/sysml-rs/reference/cli-commands/).
- **LSP** (`sysml-lsp-server`) — see [the language server](/sysml-rs/use/lsp/)
  and [editors](/sysml-rs/use/editors/).
- **REST/WebSocket** (`sysml-api`, axum, loopback-only by default) — see
  [the service API](/sysml-rs/use/service-api/).
- **MCP** (`sysml-mcp`, one tool per command) — see
  [the MCP server](/sysml-rs/use/mcp/).

None of them reimplements logic, so behaviour cannot drift between surfaces —
and the test suite enforces it with cross-transport parity fixtures. The
machine-readable catalogue of every registered command is served at
`GET /commands` by `sysml-api` and rendered in the
[API and MCP catalogue](/sysml-rs/reference/api-mcp-catalog/).

Deep guides:
[11-sysml-service-design.md](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/11-sysml-service-design.md) ·
[10-mcp-server-architecture.md](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/10-mcp-server-architecture.md) ·
[07-lsp-architecture.md](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/07-lsp-architecture.md)

## Incrementality: the salsa analysis host

`sysml-ide-db` wraps analysis in a [salsa](https://github.com/salsa-rs/salsa)
incremental-computation database, following the rust-analyzer pattern:
`AnalysisHost` (`crates/tooling/sysml-ide-db/src/host.rs`) owns the mutable
database; it hands out immutable `Analysis` snapshots that can be queried
concurrently. `SysmlService` holds the host and locks it only briefly — to set
inputs after an edit or to take a snapshot.

What this buys: after a keystroke, tree-sitter reparses only the edited
region, and salsa recomputes only the queries whose inputs actually changed.
Parse results, resolution, elaboration, diagnostics, and cached ViewModels are
all salsa queries, so an unchanged file costs nothing on re-analysis. It is
the reason one in-process model can serve an interactive editor session and
batch CLI runs with the same code path.

## The runtime: execution, verification, physics

`sysml-runtime` lowers an elaborated `ModelGraph` into its own execution IR
(`src/compiler/`) and executes it:

- **Discrete behaviour** — state machines, actions, message/flow exchange
  (`src/actions/`, `src/flows/`).
- **Expressions, constraints, and cases** — expression evaluation, constraint
  checking to verdicts, calculation/analysis/verification cases
  (`src/expressions/`, `src/constraints.rs`, `src/cases/`).
- **Continuous dynamics** — ODE/DAE integration via the `diffsol` solver, and
  hybrid discrete-plus-continuous stepping (`src/hybrid.rs`), plus Monte Carlo
  and sweep tooling on top.

This is honestly the **youngest part of the codebase**. The models under
`examples/` are what the regression suite locks down; novel model shapes will
find gaps, and interfaces here move fastest. The user-facing view of the same
machinery is on [executing models](/sysml-rs/use/runtime/) and
[the Simulation App](/sysml-rs/use/simulation-app/).

Deep guide: [06-execution.md](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/06-execution.md)

## Diagrams: ViewModel is the wire contract

Diagrams are server-rendered, never parsed or laid out from scratch in the
browser:

```text
ModelGraph → ViewRequest → DiagramIR → ViewModel → React-SVG renderer / exports
```

`sysml-diagram` turns a view request (a standard view family plus expansion,
filter, and frame inputs) into `DiagramIR` — semantic nodes, ports, edges,
compartments — and joins it with design tokens, text map, and interaction map
into a `ViewModel` (`crates/lang/sysml-diagram/src/view_model.rs`).
**ViewModel is the only diagram wire contract**: the Simulation App's
React-SVG canvas renders it, the CLI exports it, and simulation, verdict, and
diagnostic overlays attach to it as sidecars keyed by element id rather than
mutating the scene. PlantUML export runs off the same graph.

User-facing view: [views and diagrams](/sysml-rs/use/views-and-diagrams/).
Deep guide: [09-vis-pipeline-architecture.md](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/09-vis-pipeline-architecture.md)

## Layering rules that matter to contributors

The workspace is two crate groups plus a test tier, and dependencies only
point downward:

- **`crates/lang/`** implements the SysML v2 specification itself — the
  semantic model, parser, runtime, diagram exporters, manifests. Spec-defined
  behaviour, not tooling convenience. Lang crates never depend on tooling
  crates.
- **`crates/tooling/`** builds the usable surfaces on top — service hub,
  salsa analysis, resolution of project dependencies, storage, and the four
  transports.
- **`crates/testing/`** is a leaf: it depends on everything and nothing
  depends on it.

Within each group there are numbered layers (foundations → core → parser →
features → service → transports); a lower layer never imports a higher one, so
the build resolves in one pass and you can test `sysml-core` without starting
a server. Before adding a crate, a dependency, or a cross-layer edge, read
[00-architecture.md](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/00-architecture.md)
— it is the rulebook, including where recently deleted crates went.

## Where to go next

- [Where the code lives](/sysml-rs/develop/code-map/) — the crate-by-crate map,
  with entry points and "where would I change X?" answers.
- [Developer guide index](https://github.com/RickyMillar/sysml-rs/blob/main/docs/developer_guide/README.md)
  — all twelve deep guides.
- [CONTRIBUTING.md](https://github.com/RickyMillar/sysml-rs/blob/main/CONTRIBUTING.md)
  — environment setup, generated-file rules, and what a reviewable PR looks
  like.
