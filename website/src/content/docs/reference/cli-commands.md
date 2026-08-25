---
title: CLI command reference
description: Every sysml CLI command and its options, generated from the live --help output.
scope:
  - sysml-rs tooling
status: pre-alpha
last_verified_against: 11bd751
source_of_truth:
  - website/src/generated/cli-commands.json
  - crates/tooling/sysml-cli
---

<!--
GENERATED — do not edit.
Regenerate (with the artifact it renders, src/generated/cli-commands.json) via:
  cd website && node scripts/generate-reference.mjs
-->

This is the complete command catalogue of the `sysml` CLI (`sysml 0.1.0`), captured from the binary's own `--help` output — nothing here is hand-maintained. For task-oriented walkthroughs, start at [CLI workflows](/sysml-rs/use/cli-workflows/) instead.

## Global usage

```text
Usage: sysml [OPTIONS] <COMMAND>
```

| Global option | Description |
|---|---|
| `--quiet` | Suppress progress output on stderr (also silences the progress subscriber even when stderr is a TTY). Place before the subcommand |

| Command | Summary |
|---|---|
| [`sysml init`](/sysml-rs/reference/cli-commands/#sysml-init) | Initialize a new SysML project |
| [`sysml info`](/sysml-rs/reference/cli-commands/#sysml-info) | Show project information |
| [`sysml add`](/sysml-rs/reference/cli-commands/#sysml-add) | Add a dependency to sysml.toml |
| [`sysml remove`](/sysml-rs/reference/cli-commands/#sysml-remove) | Remove a dependency from sysml.toml |
| [`sysml lock`](/sysml-rs/reference/cli-commands/#sysml-lock) | Resolve dependencies and update sysml.lock |
| [`sysml fetch`](/sysml-rs/reference/cli-commands/#sysml-fetch) | Resolve dependencies and fetch/cache all sources (without writing sysml.lock) |
| [`sysml update`](/sysml-rs/reference/cli-commands/#sysml-update) | Force dependency update and rewrite sysml.lock |
| [`sysml tree`](/sysml-rs/reference/cli-commands/#sysml-tree) | Show dependency graph |
| [`sysml why`](/sysml-rs/reference/cli-commands/#sysml-why) | Show why a dependency exists in the resolved graph |
| [`sysml cache`](/sysml-rs/reference/cli-commands/#sysml-cache) | Manage local dependency cache |
| [`sysml package`](/sysml-rs/reference/cli-commands/#sysml-package) | Build a .kpar distribution archive |
| [`sysml eval`](/sysml-rs/reference/cli-commands/#sysml-eval) | Evaluate a SysML expression |
| [`sysml check`](/sysml-rs/reference/cli-commands/#sysml-check) | Check constraints in a SysML file |
| [`sysml verify`](/sysml-rs/reference/cli-commands/#sysml-verify) | Run a verification case |
| [`sysml analysis`](/sysml-rs/reference/cli-commands/#sysml-analysis) | Run an analysis case |
| [`sysml trade-study`](/sysml-rs/reference/cli-commands/#sysml-trade-study) | Run a trade study (evaluate alternatives against an objective) |
| [`sysml simulate`](/sysml-rs/reference/cli-commands/#sysml-simulate) | Simulate a state machine |
| [`sysml run`](/sysml-rs/reference/cli-commands/#sysml-run) | Run an action |
| [`sysml solve`](/sysml-rs/reference/cli-commands/#sysml-solve) | Solve constraint network via binding propagation |
| [`sysml trace`](/sysml-rs/reference/cli-commands/#sysml-trace) | Generate a sequence trace from flow simulation |
| [`sysml flow`](/sysml-rs/reference/cli-commands/#sysml-flow) | Inspect and test port flows |
| [`sysml project`](/sysml-rs/reference/cli-commands/#sysml-project) | Manage SysML projects (legacy) |
| [`sysml query`](/sysml-rs/reference/cli-commands/#sysml-query) | Query elements in a SysML model |
| [`sysml export`](/sysml-rs/reference/cli-commands/#sysml-export) | Export a SysML model to various formats |
| [`sysml inspect`](/sysml-rs/reference/cli-commands/#sysml-inspect) | Inspect semantic tokens and diagnostics for a SysML file |

## `sysml init`

Initialize a new SysML project

```text
Usage: sysml init [OPTIONS]
```

| Option | Description |
|---|---|
| `--name &lt;NAME&gt;` | Project name (creates a new directory) |

## `sysml info`

Show project information

```text
Usage: sysml info [OPTIONS]
```

| Option | Description |
|---|---|
| `--manifest-path &lt;MANIFEST_PATH&gt;` | Path to sysml.toml (auto-discovered if omitted) |
| `--json` | Output as JSON |

## `sysml add`

Add a dependency to sysml.toml

```text
Usage: sysml add [OPTIONS] <NAME>
```

| Argument | Description |
|---|---|
| `&lt;NAME&gt;` | Dependency name |

| Option | Description |
|---|---|
| `--path &lt;PATH&gt;` | Local path dependency |
| `--git &lt;GIT&gt;` | Git repository URL |
| `--tag &lt;TAG&gt;` | Git tag |
| `--branch &lt;BRANCH&gt;` | Git branch |
| `--rev &lt;REV&gt;` | Git revision (commit hash) |
| `--kpar &lt;KPAR&gt;` | KPAR archive URL |

## `sysml remove`

Remove a dependency from sysml.toml

```text
Usage: sysml remove <NAME>
```

| Argument | Description |
|---|---|
| `&lt;NAME&gt;` | Dependency name to remove |

## `sysml lock`

Resolve dependencies and update sysml.lock

```text
Usage: sysml lock [OPTIONS]
```

| Option | Description |
|---|---|
| `--force` | Force re-resolve even if lock file is up to date |
| `--quiet` | Suppress non-error output |
| `--json` | Output as JSON |

## `sysml fetch`

Resolve dependencies and fetch/cache all sources (without writing sysml.lock)

```text
Usage: sysml fetch [OPTIONS]
```

| Option | Description |
|---|---|
| `--quiet` | Suppress non-error output |
| `--json` | Output as JSON |

## `sysml update`

Force dependency update and rewrite sysml.lock

```text
Usage: sysml update [OPTIONS]
```

| Option | Description |
|---|---|
| `--quiet` | Suppress non-error output |
| `--json` | Output as JSON |

## `sysml tree`

Show dependency graph

```text
Usage: sysml tree [OPTIONS]
```

| Option | Description |
|---|---|
| `--quiet` | Suppress non-error output |
| `--json` | Output as JSON |

## `sysml why`

Show why a dependency exists in the resolved graph

```text
Usage: sysml why [OPTIONS] <NAME>
```

| Argument | Description |
|---|---|
| `&lt;NAME&gt;` | Dependency package name |

| Option | Description |
|---|---|
| `--quiet` | Suppress non-error output |
| `--json` | Output as JSON |

## `sysml cache`

Manage local dependency cache

```text
Usage: sysml cache <COMMAND>
```

### `sysml cache clean`

Remove cached dependency artifacts

```text
Usage: sysml cache clean [OPTIONS]
```

| Option | Description |
|---|---|
| `--all` | Also remove other cache files under the cache root |
| `--json` | Output as JSON |
| `--quiet` | Suppress non-error output |

## `sysml package`

Build a .kpar distribution archive

```text
Usage: sysml package [OPTIONS]
```

| Option | Description |
|---|---|
| `--manifest-path &lt;MANIFEST_PATH&gt;` | Path to sysml.toml (auto-discovered if omitted) |
| `-o, --output &lt;OUTPUT&gt;` | Output directory (default: target/package/) |

## `sysml eval`

Evaluate a SysML expression

```text
Usage: sysml eval <EXPR>
```

| Argument | Description |
|---|---|
| `&lt;EXPR&gt;` | The expression to evaluate (e.g. "2 + 3") |

## `sysml check`

Check constraints in a SysML file

```text
Usage: sysml check [OPTIONS] <FILE>
```

| Argument | Description |
|---|---|
| `&lt;FILE&gt;` | Path to the SysML file |

| Option | Description |
|---|---|
| `--set &lt;OVERRIDES&gt;` | Override attribute values (e.g. --set mass=2600) |
| `--json` | Output results as JSON |

## `sysml verify`

Run a verification case

```text
Usage: sysml verify [OPTIONS] <CASE_NAME> <FILE>
```

| Argument | Description |
|---|---|
| `&lt;CASE_NAME&gt;` | Name of the verification case to run |
| `&lt;FILE&gt;` | Path to the SysML file |

| Option | Description |
|---|---|
| `--set &lt;OVERRIDES&gt;` | Override attribute values (e.g. --set speed=85) |
| `--json` | Output results as JSON |

## `sysml analysis`

Run an analysis case

```text
Usage: sysml analysis [OPTIONS] <CASE_NAME> <FILE>
```

| Argument | Description |
|---|---|
| `&lt;CASE_NAME&gt;` | Name of the analysis case to run |
| `&lt;FILE&gt;` | Path to the SysML file |

| Option | Description |
|---|---|
| `--set &lt;OVERRIDES&gt;` | Override attribute values (e.g. --set temperature=350) |
| `--json` | Output results as JSON |

## `sysml trade-study`

Run a trade study (evaluate alternatives against an objective)

```text
Usage: sysml trade-study [OPTIONS] <STUDY_NAME> <FILE>
```

| Argument | Description |
|---|---|
| `&lt;STUDY_NAME&gt;` | Name of the trade study analysis case |
| `&lt;FILE&gt;` | Path to the SysML file |

| Option | Description |
|---|---|
| `--set &lt;OVERRIDES&gt;` | Override attribute values (e.g. --set mass=100) |
| `--json` | Output results as JSON |

## `sysml simulate`

Simulate a state machine

```text
Usage: sysml simulate [OPTIONS] <SM_NAME> <FILE>
```

| Argument | Description |
|---|---|
| `&lt;SM_NAME&gt;` | Name of the state machine |
| `&lt;FILE&gt;` | Path to the SysML file |

| Option | Description |
|---|---|
| `--events &lt;EVENTS&gt;` | Comma-separated list of events (e.g. "timer,timer,reset") |
| `--interactive` | Interactive mode: read events from stdin |
| `--auto` | Auto-demo mode: walk all transitions automatically |
| `--trace` | Show detailed execution trace |
| `--json` | Output results as JSON |

## `sysml run`

Run an action

```text
Usage: sysml run [OPTIONS] <ACTION_NAME> <FILE>
```

| Argument | Description |
|---|---|
| `&lt;ACTION_NAME&gt;` | Name of the action to run |
| `&lt;FILE&gt;` | Path to the SysML file |

| Option | Description |
|---|---|
| `--trace` | Show detailed execution trace |
| `--json` | Output results as JSON |

## `sysml solve`

Solve constraint network via binding propagation

```text
Usage: sysml solve [OPTIONS] <FILE>
```

| Argument | Description |
|---|---|
| `&lt;FILE&gt;` | Path to the SysML file |

| Option | Description |
|---|---|
| `--set &lt;OVERRIDES&gt;` | Override attribute values (e.g. --set mass=2600) |
| `--rollup &lt;ROLLUP&gt;` | Compute rollup for a named property (e.g. --rollup mass) |
| `--sweep &lt;SWEEP&gt;` | Sweep a parameter across a range (format: param:lo:hi, e.g. --sweep speed:0:200) |
| `--json` | Output results as JSON |

## `sysml trace`

Generate a sequence trace from flow simulation

```text
Usage: sysml trace [OPTIONS] <FILE>
```

| Argument | Description |
|---|---|
| `&lt;FILE&gt;` | Path to the SysML file |

| Option | Description |
|---|---|
| `--inject &lt;INJECT&gt;` | Inject messages to simulate flow (format: source.port:value, repeatable) |
| `--json` | Output results as JSON |

## `sysml flow`

Inspect and test port flows

```text
Usage: sysml flow [OPTIONS] <FILE>
```

| Argument | Description |
|---|---|
| `&lt;FILE&gt;` | Path to the SysML file |

| Option | Description |
|---|---|
| `--flow-name &lt;FLOW_NAME&gt;` | Name of a specific flow (optional — shows all if omitted) |
| `--inject &lt;INJECT&gt;` | Inject a payload into the flow source (JSON or simple value) |
| `--json` | Output results as JSON |

## `sysml project`

Manage SysML projects (legacy)

```text
Usage: sysml project <COMMAND>
```

### `sysml project init`

Initialize a new SysML project in the current directory

```text
Usage: sysml project init [OPTIONS]
```

| Option | Description |
|---|---|
| `--name &lt;NAME&gt;` | Project name (defaults to the current directory name) |
| `--version &lt;VERSION&gt;` | Project version (defaults to "0.1.0") [default: 0.1.0] |

### `sysml project info`

Show project information

```text
Usage: sysml project info
```

### `sysml project stdlib`

List standard library projects

```text
Usage: sysml project stdlib [OPTIONS]
```

| Option | Description |
|---|---|
| `--symbols` | List all symbols exported by each library |

## `sysml query`

Query elements in a SysML model

```text
Usage: sysml query <COMMAND>
```

### `sysml query find`

Find elements by name pattern

```text
Usage: sysml query find [OPTIONS] --name <NAME> <FILE>
```

| Argument | Description |
|---|---|
| `&lt;FILE&gt;` | Path to the SysML file |

| Option | Description |
|---|---|
| `--name &lt;NAME&gt;` | Name pattern to search for (substring match) |
| `--kind &lt;KIND&gt;` | Filter by element kind (e.g. PartUsage, RequirementUsage) |
| `--json` | Output as JSON |

### `sysml query stats`

Show element statistics

```text
Usage: sysml query stats [OPTIONS] <FILE>
```

| Argument | Description |
|---|---|
| `&lt;FILE&gt;` | Path to the SysML file |

| Option | Description |
|---|---|
| `--json` | Output as JSON |

### `sysml query trace`

Show traceability matrix (requirements to parts via Satisfy)

```text
Usage: sysml query trace [OPTIONS] <FILE>
```

| Argument | Description |
|---|---|
| `&lt;FILE&gt;` | Path to the SysML file |

| Option | Description |
|---|---|
| `--json` | Output as JSON |

### `sysml query unverified`

Show unverified requirements

```text
Usage: sysml query unverified [OPTIONS] <FILE>
```

| Argument | Description |
|---|---|
| `&lt;FILE&gt;` | Path to the SysML file |

| Option | Description |
|---|---|
| `--json` | Output as JSON |

## `sysml export`

Export a SysML model to various formats

```text
Usage: sysml export <COMMAND>
```

### `sysml export plantuml`

Export model as PlantUML diagram

```text
Usage: sysml export plantuml [OPTIONS] <FILE>
```

| Argument | Description |
|---|---|
| `&lt;FILE&gt;` | Path to the SysML file |

| Option | Description |
|---|---|
| `--view &lt;VIEW&gt;` | Diagram view type [default: general] [possible values: general, state, action, sequence] |

### `sysml export json`

Export model as canonical JSON

```text
Usage: sysml export json [OPTIONS] <FILE>
```

| Argument | Description |
|---|---|
| `&lt;FILE&gt;` | Path to the SysML file |

| Option | Description |
|---|---|
| `--pretty` | Pretty-print the JSON output |

### `sysml export viewmodel`

Export a declared view's ViewModel JSON (scene + tokens + text-map + interactions + frame / non-graph payload), sidecars pruned to the view's referenced ids

```text
Usage: sysml export viewmodel [OPTIONS] --workspace <WORKSPACE> --view <VIEW>
```

| Option | Description |
|---|---|
| `--workspace &lt;WORKSPACE&gt;` | Workspace directory to load (declared views render against the whole workspace) |
| `--view &lt;VIEW&gt;` | Qualified name of the declared view to export (e.g. ShowcaseViews::OverviewView; a unique bare name also resolves) |
| `--expand-all` | Expand every expandable node |
| `--expand &lt;EXPAND&gt;` | Element id to render expanded (repeatable) |
| `-o, --output &lt;OUTPUT&gt;` | Write the JSON to this file instead of stdout |

## `sysml inspect`

Inspect semantic tokens and diagnostics for a SysML file

```text
Usage: sysml inspect [OPTIONS] [FILE]
```

| Argument | Description |
|---|---|
| `[FILE]` | Path to the SysML file (required unless --workspace is used) |

| Option | Description |
|---|---|
| `--tokens` | Show only semantic tokens |
| `--diagnostics` | Show only diagnostics |
| `--cst` | Show raw CST (tree-sitter parse tree) |
| `--json` | Output as JSON |
| `--no-stdlib` | Disable loading the standard library for inspect diagnostics |
| `--library-path &lt;LIBRARY_PATH&gt;` | Override standard library path (directory containing library.kernel/library.systems) |
| `--workspace &lt;WORKSPACE&gt;` | Inspect all files in a workspace directory with cross-file resolution |
| `--focus &lt;FOCUS&gt;` | Focus diagnostics on a specific file within workspace mode |
| `--no-workspace-deps` | Disable dependency source hydration in workspace mode |

## How this page is generated

This page and its data artifact were generated by `node scripts/generate-reference.mjs` (run from `website/`) at sysml-rs commit `11bd751` on 2026-08-25. Input: `sysml 0.1.0` at `target/release/sysml`; the full raw help text per command is stored in the artifact.
Do not edit the page by hand — regenerate it. `npm run gen-check` reports drift between the committed artifacts and a fresh generation.
