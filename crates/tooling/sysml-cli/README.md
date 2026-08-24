# sysml-cli

The `sysml` command-line binary — a thin shell over `sysml-service` exposing parse, inspect, check, simulate, solve, query, export, and project/dependency management across ~30 subcommands.

`Layer 5 · tooling` · `CLI front-end` · `bin: sysml` · `clap derive` · `feature: server (optional)`

``

## Overview

`sysml-cli` builds the `sysml` binary — the top-level integration front-end for the SysML v2 toolchain. It is a **thin command shell**: argument parsing and stdout/stderr formatting live here, but parsing and all model semantics are delegated to `sysml-service` (the salsa-backed unified hub that also serves the LSP, REST API, and MCP transports). The CLI owns file I/O and presentation; the service owns the model.

**What it owns.**

- clap command surface (`src/main.rs`) + per-command modules

- Structured exit codes (`CliError` / `ExitCode`)

- Human + `--json` rendering of service results

- stderr progress subscriber (`src/progress.rs`)

- Dependency-management UX (init/add/lock/fetch/tree/package)

**What it delegates.**

- Parse / elaborate / resolve → `sysml-service` (tree-sitter)

- Execution, constraints, flows, physics → `sysml-runtime`

- Diagram (ViewModel / PlantUML) rendering → `sysml-diagram`

- Dependency resolution → `sysml-resolve` / `sysml-project`

- REST server (optional `server` feature) → `sysml-api`

>  **Post-cleanup architecture.** The CLI no longer parses directly: it routes through `sysml-service` end-to-end. The Pest `sysml-parser-batch` crate is **gone**; tree-sitter (`sysml-parser-incremental`) is the sole parser. `sysml-analysis-ir` has been folded into `sysml-runtime`. Diagrams are server-rendered (no in-browser WASM engine). The one remaining direct parser callsite is `inspect --cst`, which calls `TreeSitterParser` for a raw CST dump.

## Where it sits

```text
entry sysml-cli (bin: sysml)
▼ dispatches every command to
hub sysml-service sysml-api (feat: server) sysml-diagram
▼ which builds on
core sysml-runtime sysml-parser-incremental sysml-resolve sysml-project sysml-manifest sysml-core
```

## Command catalogue

~30 subcommands across 6 groups, all defined as clap variants in `src/main.rs` and dispatched to equally-named modules in `src/`. Filter or sort the table below; run `sysml <cmd> --help` for the authoritative per-command flag list.

| Group | Command | Module | Key flags | What it does |
|---|---|---|---|---|
| Analysis | `inspect <file>` | inspect.rs | --tokens --diagnostics --cst --json --no-stdlib --library-path --workspace --focus --no-workspace-deps | Primary debug tool: semantic tokens, diagnostics, raw CST dump; single-file or whole-workspace with cross-file resolution. |
| Analysis | `check <file>` | check.rs | --set k=v --json | Evaluate constraints with optional attribute overrides. |
| Analysis | `verify <case> <file>` | verify.rs | --set k=v --json | Run a named verification case (exit 3 on failure). |
| Analysis | `analysis <case> <file>` | analysis.rs | --set k=v --json | Run a named analysis case. |
| Analysis | `trade_study <study> <file>` | trade_study.rs | --set k=v --json | Evaluate alternatives against an objective (trade-study analysis case). |
| Execution | `eval <expr>` | eval.rs | — | Evaluate a standalone SysML expression (e.g. `2 + 3`). |
| Execution | `simulate <sm> <file>` | simulate.rs | --events e1,e2 --interactive --auto --trace --json | Drive a state machine: scripted events, stdin-interactive, or auto-walk. |
| Execution | `run <action> <file>` | run.rs | --trace --json | Execute a named action. |
| Port sim | `solve <file>` | solve.rs | --set k=v --rollup PROP --sweep p:lo:hi --json | Constraint-network solve via binding propagation; rollups and parameter sweeps. |
| Port sim | `trace <file>` | trace.rs | --inject src.port:value (repeatable) --json | Generate a sequence trace from a flow simulation. |
| Port sim | `flow <file>` | flow.rs | --flow-name NAME --inject PAYLOAD --json | Inspect ports / run flow diagnostics; inject a payload into a flow source. |
| Query | `query find <file>` | query.rs | --name PATTERN --kind KIND --json | Find elements by name substring, optionally filtered by kind. |
| Query | `query stats <file>` | query.rs | --json | Element / relationship counts by kind. |
| Query | `query trace <file>` | query.rs | --json | Requirement→part traceability matrix (via Satisfy). |
| Query | `query unverified <file>` | query.rs | --json | List requirements with no verification. |
| Export | `export plantuml <file>` | export.rs | --view general\|state\|action\|sequence | Emit a PlantUML diagram for the chosen view. |
| Export | `export json <file>` | export.rs | --pretty | Canonical model JSON. |
| Export | `export viewmodel` | export.rs | --workspace DIR --view NAME --expand-all\|--expand ID -o FILE | A declared view's renderer-agnostic ViewModel JSON (sidecars pruned to the scene) for the diagram viewer / fixtures. |
| Project | `init` | init.rs | --name NAME | Scaffold a new project with `sysml.toml`. |
| Project | `info` | info.rs | --manifest-path PATH --json | Show manifest metadata. |
| Project | `add <name>` | add.rs | --path --git --tag --branch --rev --kpar | Add a dependency to `sysml.toml`. |
| Project | `remove <name>` | remove.rs | — | Remove a dependency from `sysml.toml`. |
| Project | `lock` | lock.rs | --force --quiet --json | Resolve dependencies and write `sysml.lock`. |
| Project | `fetch` | fetch.rs | --quiet --json | Resolve + cache all sources without writing the lock file. |
| Project | `update` | update.rs | --quiet --json | Force dependency update and rewrite `sysml.lock`. |
| Project | `tree` | tree.rs | --quiet --json | Show the resolved dependency graph. |
| Project | `why <name>` | why.rs | --quiet --json | Explain why a dependency is in the resolved graph. |
| Project | `cache clean` | cache.rs | --all --quiet --json | Remove cached dependency artifacts. |
| Project | `package` | package.rs | --manifest-path PATH -o/--output DIR | Build a `.kpar` distribution archive. |
| Legacy | `project init\|info\|stdlib` | project.rs | --name --version --symbols | Legacy project group; prefer top-level `init`/`info`. Retained for `project stdlib` (lists standard-library projects/symbols), which has no top-level equivalent. |
| Server | `serve` | serve.rs | --port (3000) --host (127.0.0.1) | Start the REST API server, loopback only by default; a non-loopback `--host` warns on startup. **Feature-gated**: only compiled with `--features server`. |

## Cross-cutting flags & conventions

**`--json` almost everywhere.**

Nearly every command emits machine-readable JSON on stdout with `--json`; otherwise it prints a human-readable summary.

**Global progress flags.**

Top-level (place *before* the subcommand): `--quiet` suppresses the stderr progress renderer; `--force-progress` (hidden, or `SYSML_FORCE_PROGRESS=1`) forces it on a non-TTY. Progress is stderr-only, so piped JSON on stdout stays clean.

**`--set key=value` overrides.**

`check`, `verify`, `analysis`, `trade_study` and `solve` accept repeatable runtime attribute overrides parsed by `common::parse_key_val`.

### Exit codes

| Code | Variant | Meaning |
|---|---|---|
| 0 | — | Success. |
| 1 | `ExitCode::UserError` | Bad input, parse failure, missing element. |
| 2 | `ExitCode::InternalError` | IO failure / unexpected internal state (e.g. service errors). |
| 3 | `ExitCode::VerificationFailure` | Model parsed but a constraint / verification check failed. |

Defined in `src/common.rs`; `main` exits with `e.exit_code as i32` after printing `error: …` to stderr. Scripts can branch on these.

## Usage

```
# Evaluate a standalone expression
sysml eval "2 + 3 * 4"

# Inspect diagnostics for a single file (the primary debug tool)
sysml inspect model.sysml --diagnostics

# Inspect a whole workspace with cross-file resolution, focused on one file
sysml inspect --workspace ./project --focus src/vehicle.sysml --diagnostics

# Check constraints with overrides, JSON out
sysml check model.sysml --set mass=2600 --json

# Solve a constraint network with a rollup and a parameter sweep
sysml solve vehicle.sysml --rollup mass --sweep speed:0:200

# Drive a state machine interactively with a trace
sysml simulate DoorController model.sysml --interactive --trace

# Trace a flow simulation by injecting a message
sysml trace pipeline.sysml --inject sensor.out:42 --json

# Bake a declared view's ViewModel fixture for the diagram viewer
sysml export viewmodel --workspace . --view StructuralOverview --expand-all -o fixture.json

# Dependency workflow
sysml init --name my-model
sysml add stdlib --git https://example.com/stdlib.git --tag v1
sysml lock
sysml tree

# Force progress output in CI (stderr not a TTY)
SYSML_FORCE_PROGRESS=1 sysml inspect big.sysml --diagnostics

# REST server — only when built with the optional feature
cargo run -p sysml-cli --features server -- serve --port 3000
```

## Key modules

| Module | Responsibility | Notes |
|---|---|---|
| `main.rs` | clap `Cli`/`Commands` definitions + dispatch to module `run()`s | Single source of truth for the command surface; global `--quiet`/`--force-progress`. |
| `common.rs` | `CliError`, `ExitCode`, `parse_key_val`, `apply_overrides` | `From<ServiceError>` maps service failures to exit code 2. |
| `progress.rs` | stderr progress subscriber spawned per long-running service construction | Thread exits when the `SysmlService` (and its broadcast sender) drops. |
| `inspect.rs` | tokens / diagnostics / CST / workspace inspection | Largest module; only direct `TreeSitterParser` callsite (CST dump). |
| `export.rs` | `plantuml` / `json` / `viewmodel` export + view enums | Calls into `sysml-diagram`. |
| `query.rs` | `find` / `stats` / `trace` / `unverified` | Read-only model queries. |
| `simulate.rs` | state-machine driving (events / interactive / auto) | Largest execution module. |
| `solve.rs` / `flow.rs` / `trace.rs` | constraint solve, port-flow inspection, sequence trace | The "port simulation" group. |
| `init/add/remove/lock/fetch/update/tree/why/cache/package.rs` | dependency-management sub-tool | Backed by `sysml-manifest` + `sysml-resolve` + `sysml-project`. |
| `project.rs` | legacy `project` group (init/info/stdlib) | Overlaps top-level commands; kept for `stdlib`. |
| `serve.rs` | tokio runtime + `sysml_api::run_server` | `#[cfg(feature = "server")]` only. |

## Dependencies

**Always-on.**

- `sysml-service` — unified hub; every command dispatches here

- `sysml-runtime` — execution, constraints, flows, physics (absorbs former `sysml-analysis-ir`)

- `sysml-parser-incremental` (feat `semantic`) — tree-sitter; direct only in `inspect --cst`

- `sysml-diagram` — PlantUML / ViewModel export (rlib; server-rendered)

- `sysml-project`, `sysml-manifest`, `sysml-resolve` — dependency commands

- `sysml-core` (feat `serde`), `sysml-span`, `sysml-parser-trait`

- `clap`, `serde`/`serde_json`, `tokio`, `thiserror`, `directories`, `sha2`, `tree-sitter`

**Optional — `server` feature.**

`server = ["dep:sysml-api"]`. The default build has **no REST server**: `sysml-api` is an optional dependency and the `serve` subcommand / `serve.rs` module are `#[cfg(feature = "server")]`-gated. Build with `--features server` to enable.

## Pitfalls & invariants

> ⚠  **Two parse paths in one binary.** Everything routes through `sysml-service` *except* `inspect --cst`, which calls `TreeSitterParser::new().parse_tree()` directly. Keep this the only exception — new commands must go through the service.

- **Each command builds its own `SysmlService`.** There is no shared/cached session, so multi-step workflows re-parse (cold stdlib load can be slow). Acceptable for a CLI; noted for the architecture review.

- **Keep `--json` support** when adding subcommands — it is the contract for scripted/CI consumers and the spec-test harness.

- **Progress is stderr-only.** Never write progress to stdout; `--json` output must stay parseable.

- **The `project` group is legacy.** Do not add new functionality there; promote to a top-level command instead.

Part of the [sysml-rs](../../../README.md) workspace · regenerated 2026-06-03
