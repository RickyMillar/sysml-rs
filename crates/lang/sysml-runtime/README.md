# sysml-runtime

The SysML v2 execution engine: compiles a `ModelGraph` into domain intermediate representations and runs them — discrete behaviour, continuous physics, and analysis — under a single orchestrated tick loop.

`Layer 3 · lang` · `execution + IR + physics` · `crate-type: rlib` · `33 pub modules` · `~1,125 tests`

## Overview

`sysml-runtime` is the model execution layer of sysml-rs. It takes a fully-resolved `sysml_core::ModelGraph` and produces executable intermediate representations (IR), then drives them through a deterministic tick loop. It owns three concerns that used to be split across separate crates:

**Behavioural.**

State machines, actions, flows, constraints, verification/analysis cases, and the expression evaluator that underpins every guard and condition.

**Continuous dynamics.**

ODE/DAE solvers (RK45, BDF via `diffsol`), zero-crossing events, hybrid (discrete↔continuous) coordination, occurrences and clocks.

**Analysis & orchestration.**

Parametric solving, physics/bond-graph causality, Monte Carlo, causation (fault trees), sequence tracing, snapshots, time series, and the multi-subsystem orchestrator.

>  **Consolidation history.** This crate is the result of the *de-fork* refactor (commit `8cae7be5`). The former `sysml-analysis-ir` crate — a diverged near-duplicate of the runtime carrying the expression+calculations and physics layers — was collapsed back into `sysml-runtime` so one crate owns the full IR + execution + physics stack (~13k LoC deduped). In the same series, `sysml-diagram` dropped its WASM `cdylib` and now depends on the full runtime (commit `0febeb61`). The deleted Pest crate `sysml-parser-batch` is gone — tree-sitter (`sysml-parser-incremental`) is the sole parser.

## Where it sits

```text
consumers sysml-service sysml-lsp-server sysml-cli sysml-api sysml-ide-db sysml-diagram
▲ depend on
Layer 3 sysml-runtime
▼ depends on
Layers 1–2 sysml-core sysml-span diffsol regex thiserror
```

The runtime is library-only (`rlib`). Its non-optional `diffsol` dependency (the DAE/ODE solver) cannot target `wasm32`, which is precisely why `sysml-diagram` — now a downstream consumer — can no longer be compiled to a browser WASM `cdylib`; diagrams are server-rendered through the LSP instead.

## Execution pipeline

Every domain follows the same shape. `ModelCompiler` (in `compiler.rs`) is the single canonical entry point — service, CLI and LSP never inline elaboration + compilation themselves.

```text
in ModelGraph → ModelCompiler::new → domain IR → Orchestrator → TickSnapshot · Output
```

The orchestrator holds a vector of `Box<dyn Executor>` — one per subsystem (state machine, action graph, ODE solver, discrete solver, physics). Each tick runs executors in `ExecutionPhase` order (`Physics → ContinuousDynamics → DiscreteDynamics → …` before state machines) so continuous state is settled before discrete guards read it.

## Core public API

#### `ModelCompiler — the compilation pipeline · src/compiler.rs`

Canonical `ModelGraph → IR` entry point. Construction elaborates the graph; the `compile_*` / `build_*` methods produce domain IR and assembled orchestrators.

```
use sysml_runtime::compiler::ModelCompiler;

let compiler = ModelCompiler::new(graph);          // elaborates
let sm  = compiler.compile_state_machine("Lamp")?; // -> StateMachineIR
let act = compiler.compile_action("Brew")?;        // -> ActionGraphIR
let cs  = compiler.compile_constraints();          // -> PrecompiledConstraintSet
if let Some(ode) = compiler.detect_ode() { /* continuous dynamics present */ }
let mut orch = compiler.build_orchestrator(/* … */)?;
```

Errors surface as `CompileError`, a runtime-native type wrapping `Vec<sysml_span::Diagnostic>`. Other entry points: `build_workspace_orchestrator`, `build_orchestrator_explicit`, `detect_ode_from_ssr`, `run_simulation`.

#### `Executor — orchestrator subsystem trait · src/orchestrator.rs`

The extensibility seam. Each subsystem kind implements `Executor`; adding a new kind means implementing this trait — no enum to edit.

```
pub trait Executor {
    fn phase(&self) -> ExecutionPhase;
    fn tick(&mut self, ctx: &TickContext<'_>) -> TickOutput;
    fn reset_executor(&mut self);
    fn is_completed(&self) -> bool;
    fn clone_boxed(&self) -> Box<dyn Executor>;       // REQUIRED — backs Orchestrator::fork
    fn sync_context_in(&mut self, _shared: &EvalContext) {}
    fn sync_context_out(&self, _shared: &mut EvalContext) {}
    // … plus introspection hooks for causation analysis
}
```


#### `Runner & CompileToIR — supporting traits · src/lib.rs`

`CompileToIR<T>` standardises `ModelGraph → IR` compilation, returning `Result<T, Vec<Diagnostic>>`. `Runner` is the narrower state-machine step interface (`step(event) → StepResult`) used by `StateMachineRunner` and tests; the orchestrator uses `Executor` instead.

#### `Root IR types · src/lib.rs`

Defined at the crate root, not in submodules: `StateMachineIR`, `StateIR`, `TransitionIR`, `ConstraintIR`, `RegionIR`, `TransitionActionIR` (`Simple(String)` | `Structured`), `AssignmentIR`, `StepResult`, `ParallelStepResult`, `SubsystemState`, `TickSnapshot`, `HistoryKind`, `AssignmentOp`. All IR builders use chained `.with_*()` methods.

## Module map

33 top-level `pub mod` declarations plus seven submodule trees. Grouped by lane; filter to find a file.

| Module | Lane | Responsibility |
|---|---|---|
| `expressions/` | Behavioural | Expression IR, compiler (string→ExprIR), evaluator, stdlib, unit handling, `EvalContext` — the keystone every guard/condition shares. |
| `statemachine/` | Behavioural | SM compilation + runner, parallel regions, transition-action parser, triggers (Event/After/When/PortMessage). |
| `actions/` | Behavioural | Action control-flow graph (`ActionGraphIR`), token-flow execution, Send/Accept nodes with port targets. |
| `flows/` | Behavioural | Flow routing (`FlowRouter`), port types (`PortInstanceIR`, `PortRegistry`), port compilation + diagnostics. |
| `constraints.rs` | Behavioural | Constraint extraction & evaluation; unifies `ConstraintIR`. |
| `cases/` | Behavioural | Verification & analysis case runners, verdicts (`VerdictKind`), evidence, trade studies. |
| `calculations.rs` | Behavioural | Calculation-definition evaluation (folded in from the former analysis-ir expression+calculations layer). |
| `view_condition.rs` | Behavioural | View membership / filter condition evaluation. |
| `ode.rs` | Continuous | ODE detection & integration entry (`OdeDetection`) for continuous dynamics. |
| `ode45.rs` | Continuous | Dormand–Prince RK45 adaptive-step integrator. |
| `ode_builder.rs` | Continuous | Builder assembling an ODE system from model metadata. |
| `ode_events.rs` | Continuous | Zero-crossing event detection and handling. |
| `solvers/` | Continuous | Implicit solvers; `bdf.rs` (backward-differentiation, stiff systems). |
| `hybrid.rs` | Continuous | Hybrid discrete↔continuous coordination. |
| `physics/` | Continuous | Bond-graph / network physics: causality, connection topology, constraints, DAE, executor, solver, parameter sweep. |
| `occurrence.rs` | Continuous | `Occurrence`, `OccurrenceKind`, `OccurrenceTracker` — occurrence semantics. |
| `clock.rs` | Continuous | `LocalClock`, `ClockRegistry` — time bases. |
| `solver.rs` | Analysis | Parametric solver: binding propagation, rollups, numeric bisection, DOF analysis, sensitivity sweep. |
| `solver_plugin.rs` | Analysis | `SolverPlugin` trait, capabilities, params, results/errors. |
| `solver_registry.rs` | Analysis | `SolverRegistry` — plugin registration & discovery. |
| `solver_builtins.rs` | Analysis | Built-in solvers registered through the plugin API. |
| `solver_external.rs` | Analysis | External / out-of-process solver integration. |
| `montecarlo.rs` | Analysis | Monte Carlo sampling/analysis (feature `montecarlo`: rand + rayon). |
| `causation.rs` | Analysis | Cause-and-effect / fault-tree causation metadata and analysis. |
| `sequence.rs` | Analysis | `SequenceTrace`, `SequenceTraceBuilder` — message/interaction traces. |
| `orchestrator.rs` | Orchestration | `Orchestrator`, `Executor` trait, `TickContext`/`TickOutput`, `ExecutionPhase`, `TickStrategy`, fork. |
| `compiler.rs` | Orchestration | `ModelCompiler` — the unified compilation pipeline; `CompileError`. (~4.8k lines.) |
| `breakpoint.rs` | Orchestration | `Breakpoint`, `BreakpointId`, `CompareOp` — debug/inspection breakpoints. |
| `snapshot_view.rs` | Orchestration | Snapshot projection / view of runtime state. |
| `snapshot_diff.rs` | Orchestration | Diffing between runtime snapshots. |
| `timeseries.rs` | Orchestration | Time-series accumulation across ticks. |
| `aggregates.rs` | Orchestration | Aggregate metrics over a run. |
| `session_events.rs` | Orchestration | Session-level event records emitted during a run. |

## Usage example

Compile and step a state machine directly (no orchestrator). Compiles against the current API.

```
use sysml_runtime::compiler::ModelCompiler;
use sysml_runtime::Runner;
use sysml_core::ModelGraph;

fn run(graph: ModelGraph) -> Result<(), Box<dyn std::error::Error>> {
    let compiler = ModelCompiler::new(graph);
    let ir = compiler.compile_state_machine("TrafficLight")?;

    let mut runner = sysml_runtime::statemachine::StateMachineRunner::new(&ir);
    let result = runner.step("timerExpired");
    println!("entered: {:?}", result.entered_states);
    Ok(())
}
```

>  Building a multi-subsystem orchestrator (the path used by `sysml.sessions.*`) needs an `EvalContext` seed. The seed helper lives in `sysml-ide-db` (`eval_context_seed::context_from_graph`), which is why this crate carries a **dev-dependency cycle** on `sysml-ide-db` — for tests/benches only; the library itself does not depend on it.

## Dependencies

**Upstream (library).**

- `sysml-core` — `ModelGraph`, `Element`, `Value`

- `sysml-span` — `Diagnostic`

- `diffsol` — DAE/ODE solver (non-optional; blocks `wasm32`)

- `regex`, `thiserror`

- optional: `tracing`, `rand`/`rand_distr`/`rayon` (`montecarlo`), `serde`/`serde_json`, `schemars`

**Downstream consumers.**

- `sysml-service` — unified command hub (primary production consumer)

- `sysml-lsp-server`, `sysml-cli`, `sysml-api`

- `sysml-ide-db` — salsa DB (also the dev-dep cycle counterpart)

- `sysml-diagram` — server-side diagram rendering (post de-fork)

- `sysml-spec-tests` — conformance suite

## Invariants & pitfalls

- **One `EvalContext`, one `Value`.** `EvalContext` lives in `expressions/mod.rs`; `Value` comes from `sysml-core`. Never define parallels.

- **Every `Executor` must implement `clone_boxed()`** — `Orchestrator::fork()` (session fork) depends on it.

- **Guard fallback.** If a guard string fails to compile/eval to `ExprIR`, `evaluate_guard()` silently falls back to string equality against the event name (kept for backward compatibility; flagged tech debt).

- **`PortRegistry` is optional (ADR-2).** `route_pending_with_ports(None)` falls back to string-key routing.

- **`TickStrategy` ordering (ADR-4).** `StepFirst` (default) steps then routes; `RouteFirst` only for port-triggered scenarios.

- **Expression recursion cap.** `MAX_EVAL_DEPTH = 128`; deeper nesting returns `EvaluationError::RecursionLimit`.

Part of the [sysml-rs](../../../README.md) workspace · agent guidance in `CLAUDE.md` · regenerated 2026-06-03
