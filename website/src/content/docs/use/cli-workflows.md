---
title: CLI workflows
description: Goal-oriented tour of the sysml CLI — inspecting, checking, querying, exporting, and executing SysML v2 models from the terminal.
scope:
  - sysml-rs tooling
status: pre-alpha
last_verified_against: fcd1305
source_of_truth:
  - crates/tooling/sysml-cli/src/main.rs
  - crates/tooling/sysml-cli/src/common.rs
  - crates/tooling/sysml-cli/tests/cli_integration.rs
known_limitations: /sysml-rs/reference/known-limitations/
---

You have a `.sysml` file (or a whole project) and want to work with it from the
terminal: see what the tool thinks of it, check its constraints, query its
structure, export it, or execute it. The `sysml` binary does all of that.

This page walks the common workflows rather than listing every command —
**`sysml --help` and each subcommand's `--help` are the authoritative
reference**, and they will drift ahead of this page while the tool is
pre-alpha. Every output shown here was produced by a real run against the
commit in the page footer (outputs trimmed where marked).

## Ground rules the whole CLI follows

- **Exit codes are structured** (`crates/tooling/sysml-cli/src/common.rs`):
  `0` success, `1` user error (bad input, parse failure, missing element),
  `2` internal error (IO failure, unexpected state), `3` verification or
  constraint failure — the model was checked and failed. Scripts can branch on
  "broken invocation" versus "model fails its checks".
- **`--json` is available on most read and execute commands** and prints a
  machine-readable result on stdout. Human-facing progress goes to stderr;
  `sysml --quiet <command>` (the flag goes *before* the subcommand) silences
  it for scripting.
- Commands that take a `<FILE>` parse that one file (plus the bundled SysML
  standard library). Workspace-aware commands (`sysml inspect --workspace`,
  `sysml export viewmodel --workspace`) load a directory with cross-file name
  resolution instead.

A quick feel for the error contract:

```console
$ sysml eval "1 + nope"
error: execution error: undefined variable: `nope`   # exit 1
$ sysml check /nonexistent/file.sysml
error: io error on /nonexistent/file.sysml: No such file or directory (os error 2)   # exit 2
```

## Inspect and debug a model

`sysml inspect` is the "what does the tool actually see?" command: it reports
diagnostics (and, with flags, semantic tokens or the raw parse tree) for a
file or a whole workspace directory.

```console
$ sysml inspect examples/quantity-mismatch/QuantityMismatch.sysml --diagnostics
info: standard library enabled

error[UQ001]: binding <anonymous> connects incompatible quantity dimensions: 'lengthAttr' [L] and 'massAttr' [M]
  --> examples/quantity-mismatch/QuantityMismatch.sysml:16:4
warning[UQ001]: binding <anonymous> binds dimensioned 'massAttr' [M] to untyped 'untyped' — declare its ISQ type to enable dimensional checking
  --> examples/quantity-mismatch/QuantityMismatch.sysml:24:4
```

For a multi-file project, point it at the directory instead — names resolve
across files and, when a `sysml.toml` is present, across
[dependencies](/sysml-rs/use/dependencies/):

```console
$ sysml inspect --workspace examples/view-showcase --diagnostics
info: workspace: examples/view-showcase (2 files)
info: standard library enabled

=== Model.sysml (4 diagnostics) ===
  info[FL008]: flow 'torqueFlow' has payload type 'Torque' but endpoints lack matching feature type
  ...
summary: 4 diagnostics (0 errors, 0 warnings, 4 info)
```

Useful variants (see `sysml inspect --help` for the full set): `--tokens` for
semantic tokens, `--cst` for the raw tree-sitter parse tree, `--json` for
machine-readable output, `--focus <file>` to narrow workspace diagnostics to
one file.

For a quick expression sanity check there is `sysml eval`:

```console
$ sysml eval "2 + 3"
5
```

## Check constraints and run verification cases

`sysml check` evaluates every constraint in a file against sibling attribute
values and exits `3` if any fail. Given this model:

```sysml
// rover.sysml
package Rover {
    private import ScalarValues::Real;

    part def Chassis {
        attribute mass : Real;
        attribute topSpeed : Real;
    }

    part rover : Chassis {
        attribute mass = 180;
        attribute topSpeed = 12;

        assert constraint massBudget { mass <= 150 }
        assert constraint speedFloor { topSpeed >= 10 }
    }
}
```

```console
$ sysml check rover.sysml
[PASS] speedFloor: topSpeed >= 10
[FAIL] massBudget: mass <= 150

1/2 constraints passed, 1 failed
error: one or more constraints failed   # exit 3
```

With `--json` the same run produces a structured verdict per constraint
(`verdict` is `Pass`, `Fail`, or inconclusive; condensed here):

```json
{
  "constraints": 2,
  "results": [
    { "description": "speedFloor", "instance": "rover", "satisfied": true,  "verdict": "Pass" },
    { "description": "massBudget", "instance": "rover", "satisfied": false, "verdict": "Fail" }
  ]
}
```

`sysml verify` runs a named verification case — a `verification def` with an
`objective` that contains `verify requirement` members — and reports a
per-requirement verdict. A requirement contributes a real verdict only when
it carries modeled pass criteria (`require constraint` bodies); otherwise the
case comes back `inconclusive`. A passing run (JSON condensed):

```console
$ sysml verify MassTest rover-verify.sysml --json
{
  "case": "MassTest",
  "requirements": [
    { "requirement": "massCheck", "verdict": "pass", "message": "all constraints satisfied" }
  ],
  "summary": { "pass": 1, "fail": 0, "inconclusive": 0, "error": 0, "overall": "pass" },
  "verdict": "pass"
}
```

**Known defect:** `sysml verify` currently prints an error-prefixed line and
exits `3` even when the verdict is a pass, so scripts cannot yet rely on its
exit code — check the JSON `verdict` field instead. See
[known limitations](/sysml-rs/reference/known-limitations/).

Verification cases whose constraints reference simulation-produced state
(the pattern in `examples/oscillator-tuning-study/`) are inconclusive from
the static CLI path by design — they verify against a running session's
final state. See [the runtime page](/sysml-rs/use/runtime/) for that
boundary.

## Query and export

`sysml query` runs structured queries over the semantic graph of a file:

```console
$ sysml query find --name Engine examples/view-showcase/Model.sysml
ID                                       KIND                           NAME
------------------------------------------------------------------------------------------
c10b398a-6189-4343-8541-4852baa1cd29     PartDefinition                 Engine

1 element(s) found
```

- `sysml query stats <file>` — element counts by metaclass kind.
- `sysml query find --name <pattern> [--kind PartUsage] <file>` — find
  elements by name substring, optionally filtered by kind.
- `sysml query trace <file>` — requirements-to-parts traceability matrix via
  `satisfy` relationships.
- `sysml query unverified <file>` — requirements no verification case covers:

```console
$ sysml query unverified examples/view-showcase/Model.sysml
ID                                       NAME
------------------------------------------------------------
2c4ef873-941b-43b3-990f-d272ffdedd60     safetyCheck
...
5 unverified requirement(s)
```

`sysml export` serializes the model out of the tool:

- `sysml export json <file> [--pretty]` — the canonical JSON form of the
  semantic graph (every element with id, kind, spans, and properties).
- `sysml export plantuml <file> [--view general|state|action|sequence]` — a
  PlantUML diagram of the chosen view:

```console
$ sysml export plantuml examples/valve-gating/ValveGating.sysml --view state
@startuml
hide empty description

state "open" as 6d01d9e7-...
state "closed" as 732d89c2-...
...
@enduml
```

- `sysml export viewmodel --workspace <dir> --view <QualifiedName>` — the
  ViewModel JSON (scene, tokens, interactions) for a `view` declared in the
  model; this is the same payload the diagram surfaces render. Requires
  workspace mode because declared views render against the whole workspace.

## Run and execute

These commands execute the model. The execution semantics live in the shared
runtime — the [runtime page](/sysml-rs/use/runtime/) covers what is and is
not supported; this section is just the CLI surface.

**State machines** — `sysml simulate <SmName> <file>` steps a state machine
through a comma-separated event list (`--events`), an automatic walk
(`--auto`), or interactive stdin mode (JSON condensed):

```console
$ sysml simulate ValveSM examples/valve-gating/ValveGating.sysml --events close_valve --json
{
  "completed": true,
  "final_state": "closed",
  "steps": [ { "event": "close_valve", "from": "closed", "to": "closed", "completed": true } ]
}
```

The simulation itself is exercised by the regression suite, but the state
*labels* in this command's output are currently unreliable — on fixtures with
an explicit `entry; then <state>;` the reported initial state does not always
match the modeled entry state. Treat the labels as experimental.

**Actions** — `sysml run <ActionName> <file> [--trace]` executes an action
definition step by step:

```console
$ sysml run BrewCoffee crates/tooling/sysml-cli/fixtures/test_action.sysml --trace
action: BrewCoffee
  step 1:
    perform heatWater (no library entry)
  step 2:
    Action BrewCoffee reached final node
completed after 3 step(s)
```

**Constraint networks** — `sysml solve <file>` propagates known values
through binding connectors, reports degrees of freedom, and can sweep a
parameter to find where constraint verdicts flip:

```console
$ sysml solve crates/tooling/sysml-cli/fixtures/test_whatif.sysml --sweep speed:0:200
Binding Propagation:
  iterations: 1
  solved: 3 variables
    fuelLevel = Int(40)
    mass = Int(3500)
    speed = Int(85)

DOF Analysis:
  4 equations, 3 variables (3 known, 0 free)
  DOF = -4, status: OverDetermined

Sensitivity sweep of 'speed':
  'positiveSpeed' flips at 2 (FailToPass)
  'speedLimit' flips at 100 (PassToFail)
```

**Analysis cases and trade studies** — `sysml analysis <CaseName> <file>`
runs an `analysis` case and reports its parameters, outputs, and solver
status; `sysml trade-study <StudyName> <file>` evaluates the part
alternatives nested in an analysis usage against its `objective` attribute
and picks a best candidate. Both run, but their result surfaces are the
youngest of this group — treat them as **experimental** (in an observed run
the trade-study scored every alternative `0.0000` while still selecting a
"best").

**Port flows** — `sysml flow <file>` lists the compiled port-to-port flows,
and `sysml trace <file> --inject source.port:value` pushes a message through
them and prints the resulting sequence trace:

```console
$ sysml trace examples/port-message-delivery/PortMessageDelivery.sysml --inject relay.tripOut:1
Sequence Trace:
  Lifelines: 2
    [0] relay (part)
    [1] breaker (part)
  Messages: 1
    #1 @0ms: relay -> breaker : 929aa5dd-... [Int(1)]
```

## Project and dependency workflows

`sysml init`, `info`, `add`, `remove`, `lock`, `fetch`, `update`, `tree`,
`why`, `cache`, and `package` manage projects, manifests, and dependencies.
They are covered where those concepts are explained:

- [sysml.toml](/sysml-rs/use/sysml-toml/) — the project manifest.
- [Dependencies](/sysml-rs/use/dependencies/) and
  [lock file and cache](/sysml-rs/use/lock-and-cache/).
- [Workspaces](/sysml-rs/use/workspaces/) and
  [.kpar archives](/sysml-rs/use/kpar/).

## Where the CLI stops

The CLI is the batch, single-shot surface. Long-lived execution sessions —
continuous ODE/DAE simulation, snapshots, overrides, simulation-backed
verification — live in the service layer behind the desktop app, HTTP API,
and MCP server. The [runtime page](/sysml-rs/use/runtime/) draws that
boundary; [editors](/sysml-rs/use/editors/) and
[integrations](/sysml-rs/use/integrations/) cover the other transports.
