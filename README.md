# sysml-rs

A Rust implementation of the OMG **SysML v2** specification: an incremental
parser, a semantic model, an execution and physics runtime, and the tooling
that sits on top — a CLI, a language server, REST and MCP services, a VS Code
extension, and a desktop workbench.

> ### This is a preview
>
> sysml-rs implements a **substantial subset** of SysML v2, tracked against the
> OMG specification. It is not a complete or certified implementation, it has
> not been through any OMG conformance process, and we do not claim conformance.
> Expect constructs that do not parse yet, semantics that are implemented for
> the common cases but not every corner, and interfaces that will change without
> a deprecation period while the version stays `0.x`.
>
> It is useful today for reading, checking, querying, and executing models you
> write against the subset it supports. It is not ready to be the system of
> record for a programme you cannot afford to migrate.

## What it does

Text goes in and a queryable, executable model comes out. The pipeline is
**source → tree-sitter CST → semantic graph → name resolution and validation →
execution, queries, and diagrams**.

`sysml-core::ModelGraph` is the one intermediate representation. The parser
produces it; the runtime, query engine, diagram generators, and every transport
read it. Above that sits a single service layer (`SysmlService`) that all four
transports dispatch through, so the same operation behaves identically whether
you reach it from the CLI, an editor over LSP, HTTP, or an AI agent over MCP.

What that buys you concretely:

- **Read and check models** — parse, resolve names across a multi-file project
  and the standard library, and report diagnostics with source spans.
- **Execute them** — evaluate constraints and expressions, run state machines
  and actions, integrate continuous dynamics with a DAE/ODE solver, and run
  verification cases to a verdict.
- **Query them** — structured queries over the graph, traceability between
  requirements and the elements that satisfy or verify them, and export to
  canonical JSON or PlantUML.
- **Edit them** — LSP diagnostics, semantic highlighting, completion, hover,
  and go-to-definition, in VS Code or any LSP client.

### Where it is rough

Honest about the edges, because you will hit them:

- **Language coverage is partial.** The tree-sitter grammar is derived from the
  OMG Xtext grammars, but not every production is wired through to full
  semantics. Unsupported syntax surfaces as a parse diagnostic rather than
  silently doing something else.
- **The execution runtime is the youngest part.** Continuous dynamics, hybrid
  models, and verification cases work on the models in `examples/`, and those
  examples are what the regression suite locks down. Novel model shapes will
  find gaps.
- **The desktop workbench and the diagram surfaces are in active rework.** The
  CLI and LSP are the stable surfaces; treat the app as a preview of a preview.
- **No stable Rust API yet.** The workspace crates are not on crates.io and
  depend on each other by path. Do not build against the internal crates
  expecting semver until a facade crate lands.
- **`sysml-api` is a local development server** with no authentication and
  permissive CORS. See [SECURITY.md](SECURITY.md) before you expose a port.

## Quick start

From a fresh clone. Every step is required — the build genuinely does not work
without the first two.

**You need:** a Rust toolchain (CI builds on 1.92.0; there is no declared MSRV
yet), Node.js 20 (for the tree-sitter CLI and the editor packages), a C
compiler, the usual shell tools (`git`, `curl`, `awk`, `sha256sum`), and
several GB of free disk for `target/`.

```bash
git clone https://github.com/RickyMillar/sysml-rs
cd sysml-rs
```

**1. Fetch the specification sources.** The OMG materials are not vendored in
this repository (see [Specification sources](#specification-sources) for why).
A pinned fetch script reconstructs `references/sysmlv2/` from upstream at the
exact revisions this tree is built against:

```bash
tools/fetch-references/fetch.sh
```

About 210 MB and a few minutes on a first run; re-runs skip anything that
already verifies.

This is not optional and not just for spec lookups: `sysml-core`'s `build.rs`
generates element kinds, property accessors, and the validation dispatcher from
the fetched TTL and Xtext files, so `cargo build` fails without them.

**2. Generate the parser.** The tree-sitter parser is generated, not committed —
`src/parser.c` is a single ~80 MB table-driven C file:

```bash
npm install -g tree-sitter-cli@0.26.5
cd crates/lang/sysml-parser-incremental/tree-sitter
./generate_from_xtext.sh          # keywords/operators/enums, from the Xtext grammars
tree-sitter generate --abi 14     # ABI 14 exactly — see below
cd -
```

`--abi 14` is not a style preference. The Rust `tree-sitter` crate this
workspace builds against reads ABI 14; a parser generated at ABI 15 compiles
fine and then segfaults at parse time. Generation takes a while (tens of
minutes is normal) and is a one-time cost until you change the grammar.

**3. Build.**

```bash
cargo build --release
```

Release, not debug — the physics solver is unusably slow unoptimised. This
builds the default members: everything except the Tauri desktop shell.

**4. Run something real.**

Check a model's constraints:

```bash
./target/release/sysml check examples/espresso-pump-hybrid/Physics/HydraulicConstraints.sysml
```

```
[PASS] NonNegativeThresholds: pWarning >= 0.0 and exposureTrip > 0.0
[PASS] PositiveConductance: restrictionConductance > 0.0
[PASS] RegularizedRoot: epsRoot > 0.0

3/3 constraints passed, 0 failed
```

Inspect what the tool actually saw — diagnostics, semantic tokens, and the
concrete syntax tree. This is the first command to reach for when something
behaves unexpectedly:

```bash
./target/release/sysml inspect examples/view-showcase/Model.sysml
./target/release/sysml inspect examples/view-showcase/Model.sysml --json
```

Export a model:

```bash
./target/release/sysml export plantuml examples/damped-oscillator/DampedOscillator.sysml
./target/release/sysml export json     examples/damped-oscillator/DampedOscillator.sysml
```

`sysml --help` lists the rest: project and dependency management (`init`,
`add`, `lock`, `fetch`), evaluation (`eval`, `check`, `verify`, `analysis`,
`simulate`, `run`, `solve`, `trade-study`), and inspection (`query`, `tree`,
`trace`, `flow`, `export`).

The [`examples/`](examples/) directory holds the models the regression suite
runs against, from single-file models like `damped-oscillator` and `dc-motor`
up to multi-package projects with physics, state machines, and verification
cases. The larger ones — `espresso-pump-hybrid`, `espresso-production-cell`,
`view-showcase`, `physics-diagnostics-demo` — carry a README explaining what
they exercise and how they were derived.

## What's in the box

| Component | What it is | Build it with |
|---|---|---|
| `sysml` | The CLI — parse, check, query, execute, export, manage projects | `cargo build --release -p sysml-cli` |
| `sysml-lsp-server` | Language server over stdio for any LSP client | `cargo build --release -p sysml-lsp-server` |
| `sysml-api` | REST + WebSocket server over the same service instance | `cargo build --release -p sysml-api` |
| `sysml-mcp` | MCP server exposing the model to AI agents as tools | `cargo build --release -p sysml-mcp` |
| VS Code extension | LSP client: diagnostics, highlighting, completion, hover | `cd editors/vscode && npm ci && npm run package` |
| Desktop workbench | Tauri app for driving simulation and verification sessions | `npm ci` in `editors/expression-view` first, then `cd editors/simulation-app && npm ci && npm run dev:desktop` |
| Library crates | The parser, semantic model, and runtime as Rust crates | See [`docs/developer_guide/00-architecture.md`](docs/developer_guide/00-architecture.md) |

The `expression-view` step is not optional: `simulation-app` consumes it as a
`file:` dependency and resolves `katex` through the real path, so installing
`simulation-app` first leaves the build unable to resolve it. See
[CONTRIBUTING.md](CONTRIBUTING.md) for the full front-end setup.

Internally the workspace is layered, with dependencies pointing only downward —
foundations, semantic core, text frontend, language features, tooling
infrastructure, then the service layer and its transports. That layering, and
the rule that no transport may bypass the service layer, are described in the
[architecture guide](docs/developer_guide/00-architecture.md).

## Specification sources

The OMG SysML v2 and KerML materials — the specification documents, the Xtext
grammars, and the TTL metamodel — are **fetched, not vendored**. They are
published upstream under their own terms, they are large, and mirroring them
into a source tree makes it ambiguous which revision a build was made against
and under whose licence the copy sits. `tools/fetch-references` retrieves them
at pinned revisions with checksums, so builds are reproducible and provenance
stays with the upstream publisher.

The specification is the authority for every language decision in this
implementation. Where sysml-rs and the specification disagree, that is a bug in
sysml-rs. Pilot-implementation examples and our own `examples/` corpus are
illustrations, not normative sources, and are not evidence of conformance.

## Learning SysML v2

If you are learning the language rather than the tool, start with the companion
book: **[sysmlv2-book](https://www.omg.org/spec/SysML/)**, a
plain-English guide to SysML v2 maintained alongside this project. The OMG
specification itself lives at <https://www.omg.org/sysml/sysmlv2/>.

## Contributing

Contributions are welcome — bug reports and small, well-tested fixes are the
easiest place to start. [CONTRIBUTING.md](CONTRIBUTING.md) covers environment
setup, the focused-test workflow, the rules around generated files, and the
expectation that a change to language behaviour cites the specification clause
it implements.

- [CONTRIBUTING.md](CONTRIBUTING.md) — setup, testing, and PR expectations
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) — Contributor Covenant v2.1
- [SECURITY.md](SECURITY.md) — private vulnerability reporting
- [SUPPORT.md](SUPPORT.md) — where to ask questions

## License

Licensed under either of

- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([`LICENSE-MIT`](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option. Contributions are dual-licensed on the same terms; see
[CONTRIBUTING.md](CONTRIBUTING.md#license).

That covers the contents of this repository. The OMG specification materials
that `tools/fetch-references` retrieves are not part of it and carry their own
terms from the OMG.
