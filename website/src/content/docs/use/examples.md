---
title: Examples catalogue
description: The maintained models under examples/ — what each one demonstrates, the command or test that exercises it, and how to try them yourself.
scope:
  - SysML v2 / KerML
  - sysml-rs tooling
status: pre-alpha
last_verified_against: fcd1305
source_of_truth:
  - examples/
  - crates/lang/sysml-runtime/tests/physics_examples_pipeline.rs
  - crates/lang/sysml-runtime/tests/espresso_pump_hybrid.rs
  - crates/testing/sysml-spec-tests/tests/rsc2_behavioural_baseline.rs
  - crates/testing/sysml-spec-tests/tests/rsc5_quantity_baseline.rs
---

The `examples/` directory in the repository is the fastest way to see what
sysml-rs can actually do: every model in it is a maintained fixture that the
regression suite executes, so a green test suite at your checkout means they
parse, resolve, and (where applicable) run there. The models are written in standard SysML v2 (*scope: SysML v2 /
KerML*); the commands and tests that exercise them are sysml-rs tooling.

Two shapes to know before you start:

- **Single-file examples** work directly with file commands:
  `sysml inspect <file> --diagnostics`, `sysml query stats <file>`, and the
  execution commands on the [CLI workflows page](/sysml-rs/use/cli-workflows/).
- **Workspace examples** (a directory of several files, sometimes with a
  `sysml.toml`) need workspace mode:
  `sysml inspect --workspace examples/<dir> --diagnostics`.

Where an example's real payoff is *execution* rather than inspection, the
table names the test that runs it — continuous and hybrid simulation are
driven by the regression suite and by sessions, not by a single-shot CLI
command (see [the runtime page](/sysml-rs/use/runtime/)). Each test cited
below runs as `cargo test --release -p <crate> --test <name>`; all of them
were run and passed against the commit in the footer.

## Continuous and discrete dynamics (single files)

All of these declare their dynamics with the standard-library
`StateSpaceRepresentation` pattern.

| Example | Demonstrates | Exercised by |
|---|---|---|
| `bouncing-ball` | Ball under gravity with bounce events; `ContinuousStateSpaceDynamics` action bundling two ODE states | `sysml-runtime` test `bouncing_ball_pipeline` |
| `damped-oscillator` | Damped harmonic oscillator via the simplest SSR form (direct next-state calc, Euler forward) | `sysml-service` tests `contract_execution_source_dir`, `contract_simulate` |
| `dc-motor` | Cross-domain electrical-to-mechanical coupling (back-EMF and torque constants) | `sysml-runtime` test `physics_examples_pipeline` |
| `radiation-cooling` | Stefan-Boltzmann T⁴ cooling — a stiff nonlinear ODE | `physics_examples_pipeline` |
| `coulomb-friction` | Sliding block with smoothed Coulomb friction (sign-function discontinuity handling) | `physics_examples_pipeline` |
| `three-phase-ac` | Balanced three-phase sinusoidal source (time-driven outputs) | `physics_examples_pipeline` |
| `digital-filter` | Discrete-time exponential-moving-average filter (difference equation, not an ODE) | `sysml-runtime` test `composite_ssr_pipeline` |
| `zero-crossing-event` | Zero-crossing event detection firing a threshold event during integration | `sysml-runtime` test `zero_crossing_event` |
| `quantity-snapshot-demo` | An explicit-unit ISQ quantity slot carried through a stepping model | `sysml-spec-tests` `rsc5_quantity_baseline` |

Try one (real run):

```console
$ sysml inspect examples/damped-oscillator/DampedOscillator.sysml --diagnostics
info: standard library enabled

(no diagnostics)
```

## Hybrid and large workspaces

| Example | Demonstrates | Exercised by |
|---|---|---|
| `espresso-pump-hybrid` | A reciprocating pump in one 9-file fixture: CSV-backed sampled check-valve curves, coupled oscillatory ODEs, zero-crossing event location, and a five-phase state-machine cycle | `sysml-runtime` test `espresso_pump_hybrid` (11 tests: energy conservation, located crossings, event-time convergence, relief safety scenario) |
| `espresso-production-cell` | A large synthetic production cell (24 files): deterministic workspace load, repeated-instance multiplication, coupled hydraulic/thermal physics, requirements and parameter studies at scale | `sysml-runtime` tests `espresso_cell_structure` / `espresso_cell_behaviour` / `espresso_cell_links`; `sysml-service` test `espresso_cell_service` |
| `exchange-plane-fixture` | The message-exchange plane in one workspace (signal ports, trip logic) with a `sysml.toml` | `sysml-spec-tests` `exchange_plane_fixture` |

```console
$ sysml inspect --workspace examples/espresso-pump-hybrid --diagnostics
info: workspace: examples/espresso-pump-hybrid (9 files)
...
```

Both espresso fixtures ship a README with the full tour, including expected
physics constants and scenario walk-throughs.

## State machines and port messaging

| Example | Demonstrates | Exercised by |
|---|---|---|
| `valve-gating` | A two-state valve state machine gating a flow | `sysml-service` tests `contract_simulate`, `contract_orchestrate`; CLI run shown on the [CLI page](/sysml-rs/use/cli-workflows/) |
| `port-message-delivery` | A message routed port-to-port (the receive half of the message plane) | `sysml-spec-tests` `rsc2_behavioural_baseline` |
| `port-message-send` | A state machine *sending* via a port (the send half) | `rsc2_behavioural_baseline` |
| `port-message-delivery-multi` | Two usages of one part definition each receiving independently (instance multiplication) | `rsc2_behavioural_baseline` |
| `port-message-payload` | A scalar payload value consumed by the receiver in a transition guard | `rsc2_behavioural_baseline` |
| `port-message-payload-structured` | A constructed payload (`new TripCommand(...)`) whose named field the receiver reads | `rsc2_behavioural_baseline` |
| `action-port-message-delivery` | An *action* receiver advancing past `accept ... via <port>` | `rsc2_behavioural_baseline` |

The port family works with the CLI flow tools (real run):

```console
$ sysml trace examples/port-message-delivery/PortMessageDelivery.sysml --inject relay.tripOut:1
Sequence Trace:
  Lifelines: 2
    [0] relay (part)
    [1] breaker (part)
  Messages: 1
    #1 @0ms: relay -> breaker : ... [Int(1)]
```

## Quantity and physics diagnostics

These are *deliberately wrong* models: each exists to make a specific
diagnostic fire, so seeing errors when you inspect them is the point.

| Example | Demonstrates | Exercised by |
|---|---|---|
| `quantity-mismatch` | UQ001 — a binding connecting incompatible ISQ dimensions ([L] to [M]) | `sysml-spec-tests` `rsc5_quantity_baseline` |
| `quantity-value-mismatch` | UQ001 on the value-binding form — declared dimension vs the default value expression's dimension | `rsc5_value_binding_baseline` |
| `quantity-expr-mismatch` | UQ003/UQ004 — dimension mismatches inside constraint expressions | `rsc5_quantity_baseline` |
| `quantity-signal-mismatch` | UQ002 — a signal link whose endpoints carry incompatible dimensions | `rsc5_value_binding_baseline` |
| `physics-diagnostics-demo` | The physics lint family (port classification, flow payload typing, inconclusive constraints) in one file | `sysml-spec-tests` `rsc3_exchange_baseline` |

```console
$ sysml inspect examples/quantity-mismatch/QuantityMismatch.sysml --diagnostics
error[UQ001]: binding <anonymous> connects incompatible quantity dimensions: 'lengthAttr' [L] and 'massAttr' [M]
  --> examples/quantity-mismatch/QuantityMismatch.sysml:16:4
...
```

## Verification, analysis, and views

| Example | Demonstrates | Exercised by |
|---|---|---|
| `oscillator-tuning-study` | A suspension-tuning study whose verification cases judge *simulation-produced* state (`x`, `v`) — the session-backed verification flow; statically its constraints are inconclusive by design | `sysml-spec-tests` `rsc2_behavioural_baseline`; session flow in `sysml-service` `contract_sessions_verify` |
| `view-showcase` | Every recognised `view def` / `view` feature over a compact domain model (17 view declarations) | `sysml-service` tests `contract_viewmodel`, `contract_export_viewmodel`; `sysml export viewmodel` run shown on the [CLI page](/sysml-rs/use/cli-workflows/) |
| `the-book-corpus` | Vendored, byte-identical fixtures from the [SysML v2 Book](https://rickymillar.github.io/sysmlv2-book/) — a coffee-machine system model (16 files) and a views library | various contract and baseline tests; `sysml analysis thermalCheck examples/the-book-corpus/coffee-machine/demo-analysis.sysml` runs (see CLI page) |

A note on honesty: inspecting these large workspaces reports some errors
alongside the info diagnostics. In `the-book-corpus/coffee-machine` the three
errors are `VC010` on `satisfy` of *viewpoints* — a known sysml-rs resolver
gap (a viewpoint *is* a requirement per the spec vocabulary), not a broken
model. `espresso-production-cell` reports one `S060` validation error in its
requirements corpus. The behaviour suites named above pass regardless; see
[known limitations](/sysml-rs/reference/known-limitations/) for what is
tracked centrally.

## Keeping this page honest

This catalogue is hand-grouped but not hand-verified-forever: the
authoritative statement of what runs is the test suite itself. If a claim
here disagrees with `cargo test` output at your commit, the tests win —
and please file an issue.
