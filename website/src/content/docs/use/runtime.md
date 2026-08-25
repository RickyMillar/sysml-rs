---
title: Executing models — the runtime
description: What the sysml-rs execution runtime can run today — constraints, state machines, actions, verification and analysis cases, continuous and hybrid dynamics, and sessions — and where its edges are.
scope:
  - SysML v2 / KerML
  - sysml-rs tooling
  - Experimental / partial support
status: experimental
last_verified_against: fcd1305
source_of_truth:
  - crates/lang/sysml-runtime/
  - crates/lang/sysml-runtime/tests/physics_examples_pipeline.rs
  - crates/lang/sysml-runtime/tests/espresso_pump_hybrid.rs
  - crates/tooling/sysml-service/tests/contract_sessions_create.rs
  - crates/tooling/sysml-service/tests/contract_sessions_verify.rs
  - README.md
known_limitations: /sysml-rs/reference/known-limitations/
---

You have written a model and want it to *do* something: evaluate its
constraints, step its state machines, integrate its differential equations,
and come back with a verdict on its requirements. That is what the sysml-rs
execution runtime is for. It is the youngest part of the tool — this page
says what runs today, shows real runs, and is explicit about the edges.

## One boundary to keep in mind

The **modelling patterns** on this page are standard SysML v2: state
machines, actions, calculations, constraints, requirements, verification and
analysis cases, and the `StateSpaceRepresentation` pattern for dynamics
(an OMG domain library, `Domain Libraries/Analysis/StateSpaceRepresentation.sysml`
in the standard library). A model written this way is portable to other
SysML v2 tools. *(Scope: SysML v2 / KerML.)*

**How sysml-rs executes them** — the compiler, the ODE/DAE solver, sessions,
snapshots, overrides, verdict wiring, and every command and API named here —
is sysml-rs machinery, not an OMG-standard execution semantics. Another tool
will not run your model the same way, if it runs it at all. *(Scope:
sysml-rs tooling.)*

For learning the language constructs themselves, use the
[SysML v2 Book](https://rickymillar.github.io/sysmlv2-book/).

## What executes today

### Constraints, calculations, expressions

The expression evaluator sits under everything else. `sysml check` evaluates
every constraint against sibling attribute values; `sysml eval` evaluates a
bare expression. Real run (a two-constraint model, one failing):

```console
$ sysml check rover.sysml
[PASS] speedFloor: topSpeed >= 10
[FAIL] massBudget: mass <= 150

1/2 constraints passed, 1 failed
error: one or more constraints failed   # exit 3
```

See [CLI workflows](/sysml-rs/use/cli-workflows/) for the JSON form and the
exit-code contract, and `sysml solve` for propagating values through binding
connectors and sweeping a parameter to find where verdicts flip.

### Discrete behaviour: state machines and actions

State machines simulate against an event stream, actions execute step by
step:

```console
$ sysml simulate ValveSM examples/valve-gating/ValveGating.sysml --events close_valve --json   # condensed
{ "completed": true, "final_state": "closed",
  "steps": [ { "event": "close_valve", "from": "closed", "to": "closed", "completed": true } ] }
```

The state *labels* in the simulate output are currently unreliable (the
reported initial state does not always honour the modeled `entry`
transition) — see
[known limitations](/sysml-rs/reference/known-limitations/).

### Verification cases

A `verification def` whose objective contains `verify requirement` members
runs to a verdict, provided the requirements carry modeled pass criteria
(`require constraint` bodies). The spec-canonical idiom, and a real run:

```sysml
// rover-verify.sysml
package RoverVerification {
    private import ScalarValues::Real;

    part def Chassis {
        attribute mass : Real;
    }

    part rover : Chassis {
        attribute mass = 140;
    }

    requirement def MassBudget {
        require constraint massOk { mass <= 150 }
    }

    verification def MassTest {
        subject rover;
        objective massObjective {
            verify requirement massCheck : MassBudget;
        }
    }
}
```

```console
$ sysml verify MassTest rover-verify.sysml --json   # condensed
{
  "case": "MassTest",
  "requirements": [ { "requirement": "massCheck", "verdict": "pass", "message": "all constraints satisfied" } ],
  "summary": { "pass": 1, "fail": 0, "inconclusive": 0, "error": 0, "overall": "pass" },
  "verdict": "pass"
}
```

Two honesty notes:

- **Known defect:** `sysml verify` currently prints an error-prefixed line
  and exits `3` even on a passing verdict — read the JSON `verdict` field,
  not the exit code
  ([known limitations](/sysml-rs/reference/known-limitations/)).
- A requirement without `require constraint` bodies yields `inconclusive`,
  not a pass — the runtime refuses to invent a verdict it cannot ground.

Verification against **simulation-produced state** is a separate path: in
`examples/oscillator-tuning-study/`, the requirement constraints reference
the oscillator's live `x` and `v` state variables. Checked statically they
are inconclusive *by design* (`sysml inspect` reports
`CN002: variable 'x' has no value`); the intended flow is to run the model in
a session and verify against the final-tick state. That flow is exercised by
the service contract tests
(`crates/tooling/sysml-service/tests/contract_sessions_verify.rs`) and is
reachable through the session surfaces described below, not the single-shot
CLI.

### Analysis cases and trade studies — experimental

`sysml analysis <Case> <file>` runs an `analysis` case through the solver and
reports parameters, outputs, and convergence status; `sysml trade-study`
evaluates part alternatives inside an analysis usage against its `objective`
attribute. Both execute on the repository examples, but their result
surfaces are immature — in an observed run the trade study scored every
alternative `0.0000` while still picking a "best". Treat this pair as
**experimental / partial support**.

### Continuous dynamics (ODE) — via the standard SSR pattern

Models declare dynamics with the standard-library
`StateSpaceRepresentation` pattern: an action specializing
`ContinuousStateSpaceDynamics` (or the discrete/direct variants) with `out`
state attributes and `calc` members for the derivatives. The runtime detects
that structure, compiles the ODE system, and integrates it (an RK45-family
solver, `crates/lang/sysml-runtime/src/ode45.rs`).

There is **no single-shot CLI command** that drives a continuous simulation
end to end; continuous runs happen inside sessions (below) and in the
regression suite. The claim that these examples execute is locked by tests
run against this commit:

```console
$ cargo test --release -p sysml-runtime --test physics_examples_pipeline
test test_dc_motor_ode_detected ... ok
test test_dc_motor_simulation ... ok
test test_radiation_cooling_ode_detected ... ok
test test_radiation_cooling_simulation ... ok
test test_coulomb_friction_ode_detected ... ok
test test_coulomb_friction_simulation ... ok
test test_three_phase_ac_ode_detected ... ok
test test_three_phase_ac_simulation ... ok

test result: ok. 8 passed; 0 failed
```

The [examples catalogue](/sysml-rs/use/examples/) lists the continuous-physics
models (`dc-motor`, `radiation-cooling`, `coulomb-friction`,
`three-phase-ac`, `bouncing-ball`, `damped-oscillator`, and friends) with the
test that locks each one.

### Hybrid models: events, sampled data, coordination

The hybrid layer combines continuous states with zero-crossing event
location, CSV-backed sampled functions, and state-machine coordination.
`examples/espresso-pump-hybrid/` exercises all of it in one fixture; its
suite passes at this commit:

```console
$ cargo test --release -p sysml-runtime --test espresso_pump_hybrid
test result: ok. 11 passed; 0 failed
```

(The tests cover finite derivatives, energy conservation, located crossings
driving transitions, event-time convergence as the step shrinks, and the
relief-cycle safety scenario.)

## Sessions, runs, snapshots, overrides

*(Scope: sysml-rs tooling — none of this is OMG-standard.)*

A **session** is a long-lived execution of a workspace: the runtime compiles
the model once, then advances it tick by tick, recording state. On top of
sessions the service layer provides:

- **runs and snapshots** — step a session forward, capture and diff
  point-in-time state, and read back time series of any recorded variable;
- **overrides** — fork a session with modified attribute values to compare
  design variants without editing the model;
- **simulation-backed verification** — evaluate verification cases against a
  running session's state (the `oscillator-tuning-study` flow above);
- **injection** — push events and port messages into a running model.

Sessions are *not* exposed by the single-shot `sysml` CLI. They are the
execution surface of the desktop workbench, the local HTTP API, and the MCP
server — every transport dispatches into the same service layer, so the
semantics are identical. The behaviour is pinned by the service contract
tests (`crates/tooling/sysml-service/tests/contract_sessions_create.rs`,
`contract_sessions_verify.rs`, and neighbours). See
[integrations](/sysml-rs/use/integrations/) for reaching them over HTTP and
MCP.

## Limitations

Honest edges, current at the commit in the footer:

- **The runtime is the youngest part of sysml-rs.** Continuous dynamics,
  hybrid models, and verification cases work on the repository examples,
  which the regression suite locks down. Novel model shapes will find gaps —
  expect hard errors rather than silent wrong answers where the runtime
  notices, but do not assume every construct executes.
- **Simplified physics, not a calibrated twin.** The solver integrates the
  equations you wrote; it does not validate them against reality.
- **`sysml verify` exit code** is wrong on passing verdicts (defect, above).
- **`sysml simulate` state labels** can disagree with the modeled entry
  state (defect, above).
- **Attribute overrides via `--set`** on the check path did not affect
  constraint results in observed runs; prefer session forking with overrides
  for what-if work.
- **Analysis cases and trade studies** run but have immature result
  surfaces (experimental, above).

The [known limitations](/sysml-rs/reference/known-limitations/) page tracks
these centrally.
