# Execution Runtime

This guide covers the sysml-rs execution layer: what "running" SysML v2 means, what the `sysml-runtime` modules do, and how transports drive execution through `SysmlService` (S2).

> Originally written Apr 11 2026 as a roadmap; refreshed 2026-05-20 to reflect what shipped: all eight EX-1..EX-8 phases are live; phases 10–15 (port flow, parametric solving, reactive ports, activity flow, plugins, ODE) are live; the physics-aware layer landed Apr 12–13 2026 (ISQ inference, hybrid executor, dynamic topology, domain solvers, PH001–PH006 diagnostics). See [00-architecture.md](00-architecture.md) for the layering view and [20-sysml-service-design.md](20-sysml-service-design.md) for the unified-service command surface that all transports dispatch through.

## What Does "Running" SysML Mean?

SysML v2 defines **operational semantics** through KerML Performances. Unlike traditional programming languages where "run" means executing imperative code, SysML execution covers 7 distinct domains:

| Domain | What Happens | Output | Module |
|--------|-------------|--------|--------|
| Expression Evaluation | Compute `2 + 3`, `speed * 0.85` | Scalar values | `sysml-runtime::expressions` |
| Constraint Checking | Evaluate `mass < 2500kg` | Pass/Fail + violations | `sysml-runtime::constraints` |
| State Machine Execution | Step through states on events | Event traces | `sysml-runtime::statemachine` |
| Action Execution | Run sequential/parallel control flow | Step logs, messages | `sysml-runtime::actions` |
| Flow Routing | Route data between participants | Message delivery log | `sysml-runtime::flows` |
| Verification Cases | Run tests, check requirements | Verdicts (Pass/Fail) | `sysml-runtime::cases` |
| Analysis Cases | Compute derived properties | Calculated values | `sysml-runtime::cases` |
| Parametric Solving | Propagation, bisection, sensitivity | Solved values, DOF | `sysml-runtime::solver` |
| ODE Continuous Sim | RK4/RK45 integration, signal inputs | Time series, thresholds | `sysml-runtime::ode`, `ode_builder` |
| Orchestration | Multi-subsystem tick loop (ODE + SM) | Snapshots, transitions | `sysml-runtime::orchestrator` |
| Hybrid Execution | Dynamic topology, gated expressions, variable impedance | Snapshots with re-routed flows | `sysml-runtime::hybrid_executor` |
| Sequence Tracing | Record interaction traces | Lifeline diagrams | `sysml-runtime::sequence` |
| Physics Analysis (static) | ISQ inference, conservation laws, domain detection | PH001–PH006 diagnostics | `sysml-core::physics` (re-exported by runtime) |

### Spec Foundation

All execution is rooted in KerML's `Performance` concept:

```
Performance (behavioral Occurrence)
├── Evaluation (functional: compute result)
│   ├── BooleanEvaluation (predicates)
│   └── LiteralEvaluation (constants)
├── StatePerformance (reactive: entry/do/exit)
│   └── TransitionPerformance (trigger→guard→effect)
├── Action (imperative: sequential/parallel steps)
│   ├── SendAction / AcceptAction (messaging)
│   ├── AssignmentAction (state mutation)
│   ├── IfThenAction / LoopAction (control flow)
│   └── ForkAction / JoinAction (concurrency)
└── Case (structured execution)
    ├── AnalysisCase (compute properties from scenarios)
    ├── VerificationCase (test requirements → verdict)
    └── UseCase (operational scenarios)
```

Key spec files:
- `Performances.kerml` — Root execution model
- `StatePerformances.kerml` — State execution: `entry [1] → do [1..*] → exit [1]`
- `TransitionPerformances.kerml` — `trigger → guard → effect → state change`
- `ControlPerformances.kerml` — If/loop/fork/join semantics
- `VerificationCases.sysml` — Verdict computation, RequirementCheck

## Architecture

### Pipeline

```
.sysml source → Parser → ModelGraph → Elaboration → ModelCompiler → IR → Executor → Output
                                       ↑                            ↑
                                  sysml-core/                 sysml-runtime
                                  elaborate/                  (per-domain modules)
                                                                    ↑
                                                          SysmlService::<command>
                                                          (RuntimeSession, ModelCompiler)
```

1. **Parse**: `.sysml` text → `ModelGraph` (elements + relationships)
2. **Elaborate**: Tag initial states, create transitions, set properties (additive, idempotent)
3. **Compile**: `sysml_runtime::compiler::ModelCompiler` lowers `ModelGraph` → domain-specific IR (`StateMachineIR`, `ActionGraphIR`, `OdeSpec`, …)
4. **Execute**: An `Executor` (`StateMachineRunner`, `ActionRunner`, `Orchestrator`, `ContinuousDynamicsExecutor`, `ModeSelectionExecutor`, …) steps the IR with an `EvalContext` providing variable bindings
5. **Output**: Step results, verdicts, messages, violations — returned as transport-neutral types via `SysmlService`

### Service-mediated execution (S2)

All transports (CLI, LSP, REST, MCP) drive execution through `#[service_command]` methods on `SysmlService`, not by calling `sysml-runtime` directly:

| Domain | Service commands | Runtime backing |
|--------|------------------|-----------------|
| Simulation | `sysml.simulate.start` / `.step` / `.stop` | `StateMachineRunner`, `Orchestrator`, `ContinuousDynamicsExecutor` + `ModeSelectionExecutor` |
| Action | `sysml.action.run` / `.start` / `.step` | `ActionRunner` |
| Orchestrator | `sysml.orchestrate.start` / `.step` / `.inject` | `Orchestrator` (ODE + SM coupling) |
| Continuous sim | `sysml.simulate.continuous.auto` / `.start` | `Rk4Solver`, `Rk45Solver` via `Orchestrator` |
| Constraint | `sysml.constraint.check`, `sysml.evaluate.constraints` | `PrecompiledConstraintSet` |
| Expression | `sysml.expression.eval` | `ExpressionEvaluator` |
| Verification | `sysml.verify`, `sysml.evaluate.verification_cases` | `VerificationRunner` |
| Analysis | `sysml.analysis`, `sysml.evaluate.analysis_cases` | Analysis case runner |
| Trade study / what-if | `sysml.whatif`, `sysml.whatif.sweep`, `sysml.trade.study` | What-if + sensitivity in service + cases |
| Solving | `sysml.solve` | `ConstraintNetwork`, plugin solvers |
| Session lifecycle | `sysml.sessions.*` | Unified `RuntimeSession` map in `SysmlService` |

Session state lives in `SysmlService::sessions: DashMap<"{uri}:{name}", RuntimeSession>` with `MAX_SESSIONS = 50`. Legacy accessors (`simulations()`, `action_sessions()`, `orchestrator_sessions()`) return views over the same map.

### Module Dependency Graph

```
sysml-runtime (traits: Runner, CompileToIR, IR types)
├── expressions (ExprIR, evaluator, string→ExprIR compiler, stdlib)
│   ├── constraints (extract, pre-compile, evaluate, monitor)
│   ├── statemachine (simple + parallel, guards, time, guard-only)
│   ├── actions (token-flow, fork/join, send/accept, port routing)
│   ├── cases (verification, use case, analysis, verdicts)
│   └── solver (propagation, bisection, DOF, sensitivity)
├── flows (router, succession, type checking)
│   └── flows/port (PortInstanceIR, PortRegistry, typed endpoints)
├── ode + ode45 (RK4 + RK45 solvers, context-aware stepping)
├── ode_builder (SysML expressions → OdeRhs closures, signal expressions)
├── ode_events (zero-crossing detection)
├── clock (universal/local clocks, time scaling)
├── solver_plugin + solver_registry (external solver API)
├── sequence (SequenceTrace, lifeline recording)
└── orchestrator (multi-subsystem tick loop, ODE↔SM coupling, port events)
```

### Elaboration Bridge

The elaboration pass in `sysml-core/src/elaborate/` bridges parser output to execution:

| Sub-pass | What it does |
|----------|-------------|
| `state_machines.rs` | Tag initial/final states, create transition relationships, tag parallel, wire entry/do/exit actions |
| `constraints.rs` | Extract constraint expressions, set assume/require roles, detect negation |
| `successions.rs` | Create ordering relationships from succession elements |
| `flows.rs` | Extract source/target endpoints, payload types |

Elaboration is **additive** (only adds properties, never removes) and **idempotent** (safe to run multiple times).

## Current Module Status

### sysml-runtime::expressions (~3600 lines)

Full expression evaluator with 20+ IR variants:

```rust
// Compile a string expression to IR
let expr = compile_simple_expression("speed * 0.85 + offset")?;

// Evaluate with context
let mut ctx = EvalContext::new();
ctx.set("speed", Value::Float(100.0));
ctx.set("offset", Value::Float(5.0));

let result = ExpressionEvaluator::new().eval(&expr, &ctx)?;
// result = Value::Float(90.0)
```

Supports: arithmetic, comparison, logical, bitwise, conditionals (`if?else`), null coalescing (`??`), collection ops (`select`, `collect`, `forAll`, `exists`), indexing (`#()`), ranges (`..`), feature chains (`a.b.c`), function calls.

### sysml-runtime::statemachine (~2000 lines)

Simple and parallel state machine execution:

```rust
// From ModelGraph
let ir = StateMachineCompiler::compile(&graph)?;
let mut runner = StateMachineRunner::new(ir);

// Step with events
let result = runner.step(Some("timer"));
assert_eq!(result.state, "Green");

// Parallel regions
let mut parallel = ParallelStateMachineRunner::new(ir);
parallel.send("powerButton");
let result = parallel.step();
```

Supports: expression guards, do-actions, enhanced triggers (`after(duration)`, `when(condition)`), time tracking, parallel regions with token-based execution.

### sysml-runtime::constraints (~1500 lines)

Constraint extraction, pre-compilation, and monitoring:

```rust
// Extract from graph
let constraints = extract_and_precompile(&graph);

// Evaluate
let mut ctx = EvalContext::new();
ctx.set("mass", Value::Float(2400.0));
let result = evaluate(&constraints[0].constraint, &ctx);

// Continuous monitoring
let mut monitor = ConstraintMonitor::new();
monitor.add_constraint(constraint);
let violations = monitor.check(&ctx);
```

Supports: batch evaluation, short-circuit, requirement semantics (assume/require), vacuous satisfaction, negation, violation accumulation.

### sysml-runtime::actions (~2300 lines)

Token-flow action execution engine:

```rust
let ir = compile_action("BakeACake", &graph)?;
let mut runner = ActionRunner::new(ir);
let ctx = EvalContext::new();

loop {
    let result = runner.step(&ctx);
    for msg in &result.messages {
        println!("Message to {}: {}", msg.target, msg.payload);
    }
    if result.completed { break; }
}
```

Supports: 12 node types (Initial, Final, Perform, Send, Accept, Assign, If, WhileLoop, ForLoop, Terminate, Decision, Merge, Fork, Join), nested sub-actions via library, guard evaluation on edges.

### sysml-runtime::flows (~1200 lines)

Message routing between flow participants:

```rust
let flows = compile_flows(&graph)?;
let mut router = FlowRouter::new();
for flow in flows { router.add_flow(flow); }

router.send("sensor.output", Value::Float(25.5));
let delivered = router.route_pending();
let msg = router.receive("controller.input");
```

Supports: multicast routing, succession ordering (blocks until source completes), payload type checking (int→float widening), event logging, action integration helpers.

### sysml-runtime::cases (~2000 lines)

Verification, use case, and analysis execution:

```rust
// Verification
let case_ir = compile_verification_case("SpeedCheck", &graph)?;
let runner = VerificationRunner::new();
let result = runner.verify(&case_ir, &ctx);
assert_eq!(result.verdict, VerdictKind::Pass);

// Use cases
let uc_ir = compile_use_case("DriveVehicle", &graph)?;
let uc_runner = UseCaseRunner::new();
let result = uc_runner.run(&uc_ir, &ctx);
```

Supports: nested sub-requirements with aggregation, setup actions, vacuous satisfaction, undefined→Inconclusive, verdict kinds (Pass/Fail/Inconclusive/Error), verification methods (Inspect/Analyze/Demo/Test).

### sysml-core::physics (~Apr 13 2026)

Static physics analysis on `ModelGraph`. Used by both the LSP (diagnostics + hover enrichment + code lenses) and the runtime (domain solver selection).

```rust
use sysml_core::physics::{infer_isq_units, classify_domain, detect_conservation_laws};

let units = infer_isq_units(&graph);            // attribute → ISQ unit
let domain = classify_domain(&part_id, &graph); // Electrical / Thermal / Mechanical / Fluidic
let laws = detect_conservation_laws(&graph);    // KCL, KVL, Newton, thermal balance
```

Diagnostics: PH001 (port direction mismatch), PH002 (domain mixing), PH003 (unit inconsistency), PH004 (conservation violation), PH005 (missing domain annotation), PH006 (Real instead of ISQ — has quick-fix code action). Runtime consumers re-export from `sysml-core::physics`; execution itself stays in `sysml-runtime`.

### sysml-runtime::hybrid (split executors, RSC-4.3 Wave 2, 2026-07-05)

The fused `HybridExecutor` (which coupled an inline ODE step to SM mode selection inside one
executor) is **deleted** — it had zero production callers (grep-verified; `valve-gating`,
`dc-motor`, `three-phase-ac`, and `espresso-production-cell` never used it, contra an earlier claim here).
Production dynamic-topology coupling (valve open/close, switch on/off, breaker trip re-routing flows
when SM-state-gated expressions flip a connection's effective conductance) runs through the ordinary
`Orchestrator` executor pair: **`ContinuousDynamicsExecutor`** (phase `ContinuousDynamics`,
mode-dependent `OdeRhs`, with an optional per-mode retained `OdeSpec` via `with_mode_spec` for a real
`read_slots()`) and **`ModeSelectionExecutor`** (phase `StateMachine`, a thin `Executor` delegate
over `StateMachineRunner`). The two communicate only through the shared context the orchestrator
threads between executors each tick — the SM half publishes its current state name into a
`mode_signal` context key each tick, and the ODE half reads it — never a private Rust field
reference between them.

### sysml-diagram

Visualization export (operates on ModelGraph, no IR):

```rust
let dot = to_dot(&graph);                    // General DOT
let puml = to_plantuml_state_view(&graph);   // State diagram
let json = to_cytoscape_json(&graph);        // Interactive graph
render_dot_to_svg(&dot, "output.svg")?;      // Render to file
```

## User-Facing Output Types

What users see when they "run" different model elements:

| Element | Output | Example |
|---------|--------|---------|
| Expression (`calc`) | Computed value | `power = 1200` |
| Constraint (`constraint`) | Satisfaction + detail | `PASS (2400 <= 2500)` |
| Assert (`assert`) | Pass/Fail diagnostic | `VIOLATED: mass exceeds limit by 100kg` |
| State machine (`state def`) | Event trace | `Red → timer → Green → timer → Yellow` |
| Action (`action def`) | Step log | `[1] preheatOven [2] mixIngredients [3] putInOven` |
| Verification (`verification def`) | Verdict | `SpeedCheck: PASS (1/1 requirements satisfied)` |
| Analysis (`analysis def`) | Calculated properties | `fuelEconomy = 28.5 mpg` |
| Flow (`flow`) | Delivery log | `sensor.output → controller.input: 25.5` |

## Original execution roadmap — all shipped

The original eight-phase execution plan (EX-1 through EX-8) has shipped end-to-end. The table below maps the original goals to where they live today.

| Phase | Original goal | Shipped as |
|-------|---------------|------------|
| EX-1 | LSP inline expression / constraint eval | `inlay_hints.rs`, `code_lens.rs`, `evaluation.rs`, `sysml.evaluate` command |
| EX-2 | Verification case runner | `workspace_verify.rs` + `sysml.verify` / `sysml.evaluate.verification_cases` |
| EX-3 | Live constraint monitor | `constraint_monitor.rs` (both LSP and sysml-service) |
| EX-4 | State machine viz + stepping | `sysml.simulate.start/step/stop`, diagram integration via `sysml-diagram` |
| EX-5 | Action execution trace | `sysml.action.run/start/step` + `SequenceTrace` |
| EX-6 | CLI runner | `sysml-cli` subcommands: `eval`, `check`, `verify`, `simulate`, `run`, `flow`, `trace`, `solve`, `inspect` |
| EX-7 | Analysis case framework | `cases` module + `sysml.analysis` / `sysml.evaluate.analysis_cases` |
| EX-8 | Trade studies | `sysml.whatif` / `whatif.sweep` / `trade.study` / `trade.study.ode_sweep` |

The detailed roadmap entries below are preserved for historical reference — the *what* and *why* still describe the user-facing behaviour, but the implementation has long since landed. New execution features should be added through `sysml-service` (`#[service_command]`) rather than rebuilding the EX-* pattern.

### EX-1: LSP Inline Evaluation

**Goal**: Evaluate expressions and constraints inline in the editor.

**What users see**:
```sysml
attribute voltage = 120;              // inlay hint: 120
attribute current = 10;               // inlay hint: 10
calc power { voltage * current }      // code lens: = 1200

constraint massLimit {
    chassisMass + engineMass <= 2500   // code lens: PASS (2400 <= 2500)
}
```

**Implementation**:
- Add `sysml-runtime` (expressions + constraints modules) as a dependency of `sysml-lsp-server`
- Evaluate expressions after T2/T3 resolve completes
- Surface results via inlay hints (existing infrastructure) and code lenses (new)
- Register `textDocument/codeLens` capability
- Add `sysml.evaluate` execute command for manual evaluation

**Crates touched**: `sysml-lsp-server` (new deps + handlers)

**Team potential**: Low — single crate, sequential dependency on LSP internals.

### EX-2: Verification Case Runner

**Goal**: Run verification cases from the editor via code lens.

**What users see**:
```sysml
verification def SpeedCheck {         // code lens: [Run] [PASS]
    subject vehicle;
    requirement SpeedReq {
        constraint { speed < 200 }    // PASS (80 < 200)
    }
}
```

Output panel:
```
Verification: SpeedCheck
  Subject: vehicle
  Requirement SpeedReq: PASS (speed=80 < 200)
  Overall verdict: PASS
```

**Implementation**:
- Add `sysml-runtime` (cases module) dependency to LSP
- Code lens on `VerificationCaseDefinition` elements
- Execute command handler runs `compile_verification_case()` + `VerificationRunner::verify()`
- Results in output channel + inline code lens update
- Context values from model attribute assignments

**Crates touched**: `sysml-lsp-server`, possibly `sysml-runtime::cases` (for context extraction)

**Team potential**: Medium — researcher for spec semantics + implementer for LSP wiring.

### EX-3: Constraint Monitor (Live Diagnostics)

**Goal**: Continuously evaluate constraints as the model changes, show violations as diagnostics.

**What users see**:
```sysml
part vehicle {
    attribute mass = 2600;  // warning: "Violates massConstraint (2600 > 2500)"
}

constraint massConstraint {
    vehicle.mass <= 2500    // error diagnostic: VIOLATED
}
```

**Implementation**:
- `ConstraintMonitor` already exists in `sysml-runtime::constraints`
- After T3 resolve, extract all `assert constraint` and `satisfy requirement` elements
- Evaluate constraints using resolved attribute values as context
- Emit LSP diagnostics (Warning severity) for violations
- Debounce evaluation (don't re-evaluate on every keystroke)

**Crates touched**: `sysml-lsp-server`, `sysml-runtime::constraints`

**Team potential**: Medium — constraint evaluation + LSP diagnostic pipeline are independent concerns.

### EX-4: State Machine Visualization & Stepping

**Goal**: Interactive state machine execution with visual trace.

**What users see**:
```sysml
state def TrafficLight {              // code lens: [Simulate]
    state Red;                        // highlight: ACTIVE
    state Green;
    state Yellow;
    transition first Red then Green;
}
```

Side panel:
```
TrafficLight Simulation
  Current: Red [active]
  [Send "timer"] [Step] [Reset]

  Trace:
  [0] → Red (initial)
  [1] timer → Green
  [2] timer → Yellow
```

**Implementation**:
- Add `sysml-runtime` (statemachine module) dependency to LSP
- Code lens "Simulate" on `StateDefinition` elements
- Custom LSP command `sysml.simulate.start` / `sysml.simulate.step` / `sysml.simulate.event`
- State tracked server-side in `Arc<RwLock<HashMap<Uri, SimulationState>>>`
- Visualization via `sysml-diagram` PlantUML/Sprotty export, updated on each step
- Highlight active state via custom semantic token or decoration

**Crates touched**: `sysml-lsp-server`, `sysml-runtime`, `sysml-diagram`

**Team potential**: High — visualization, LSP wiring, and runner integration are independent.

### EX-5: Action Execution Trace

**Goal**: Step through action definitions, see control flow and message passing.

**What users see** (output panel):
```
Action: BakeACake
  [1] Execute: preheatOven
  [2] Execute: mixIngredients
  [3] Execute: putInOven
  Completed in 3 steps. Messages sent: 0
```

For complex actions with control flow:
```
Action: ComputeMotion
  [1] Loop iteration 1: power=100W → position=5.2m
  [2] Loop iteration 2: power=120W → position=11.8m
  Completed. Output: positions=[5.2, 11.8]
```

**Implementation**:
- Add `sysml-runtime` (actions module) dependency to LSP
- Code lens "Run" on `ActionDefinition` elements
- Step-through execution with output channel logging
- Message integration with `sysml-runtime::flows` for send/accept actions

**Crates touched**: `sysml-lsp-server`, `sysml-runtime` (actions + flows modules)

**Team potential**: Medium — action runner + flow integration can be parallel.

### EX-6: CLI Runner

**Goal**: Command-line tool for running SysML models outside an IDE.

```bash
# Evaluate an expression
sysml eval "2 + 3 * 4"
# → 14

# Check constraints
sysml check constraints.sysml --set mass=2600
# → VIOLATED: massConstraint (2600 > 2500)

# Run verification
sysml verify SpeedCheck vehicle.sysml --set speed=80
# → SpeedCheck: PASS

# Simulate state machine
sysml simulate TrafficLight vehicle.sysml --events timer,timer,timer
# → Red → Green → Yellow → Red

# Run action
sysml run BakeACake recipe.sysml
# → [1] preheatOven [2] mixIngredients [3] putInOven → Completed
```

**Implementation**:
- New `sysml-cli` binary crate (or extend existing if one exists)
- Subcommands: `eval`, `check`, `verify`, `simulate`, `run`
- Parse → elaborate → compile → execute pipeline per subcommand
- JSON and human-readable output formats
- `--set key=value` for providing context variables
- `--trace` flag for detailed execution logging

**Crates touched**: New `sysml-cli` crate, depends on `sysml-runtime`

**Team potential**: High — each subcommand is independent, can be built in parallel.

### EX-7: Analysis Case Framework

**Goal**: Execute analysis cases to compute derived properties from scenarios.

```sysml
analysis def FuelEconomy {
    subject vehicle;
    in attribute scenario : WayPoint[*];

    calc fuelConsumption { /* dynamics equations */ }
    return fuelEconomyResult;
}
```

Output:
```
Analysis: FuelEconomy
  Scenario: cityDriving (42 waypoints)
  Fuel consumption: 8.2 L/100km
  Result: fuelEconomyResult = 28.7 mpg
```

**Implementation**:
- Complete `AnalysisCaseIR` runner in `sysml-runtime::cases`
- Input binding: scenarios → context variables
- Calculation execution via `sysml-runtime::expressions`
- Result extraction and display
- Integration point for external solvers (future)

**Crates touched**: `sysml-runtime` (cases + expressions modules)

**Team potential**: Medium — analysis runner + expression expansion are separable.

### EX-8: Trade Study Support

**Goal**: Evaluate design alternatives against objectives, rank and select.

```sysml
analysis def EngineTradeStudy {
    subject vehicle;
    objective { maximize performance; minimize cost; }

    alternative engine4cyl : Engine { /* config */ }
    alternative engine6cyl : Engine { /* config */ }
}
```

Output:
```
Trade Study: Engine Selection
  engine4cyl: score 7.3 (perf=6.5, cost=8.1)
  engine6cyl: score 8.1 (perf=9.0, cost=7.2)
  Selected: engine6cyl
```

**Implementation**:
- Extend `AnalysisCaseIR` with alternatives and objective functions
- Multi-criteria evaluation (weighted sum, Pareto front)
- Ranking and selection logic
- Visualization of trade space

**Crates touched**: `sysml-runtime::cases`, possibly new `sysml-runtime::trade` module

**Team potential**: Medium — scoring logic + visualization are independent.

## Recommended Build Order

```
EX-1  Inline Evaluation     ←── immediate LSP value, connects existing crates
EX-2  Verification Runner   ←── verdicts are the most "SysML" output
EX-3  Constraint Monitor    ←── live diagnostics, huge IDE value
EX-4  State Machine Viz     ←── interactive, visual, compelling
EX-6  CLI Runner            ←── non-IDE users, CI/CD, scriptable
EX-5  Action Trace          ←── builds on state machine work
EX-7  Analysis Framework    ←── requires solver design decisions
EX-8  Trade Studies         ←── builds on analysis
```

### Phase Dependencies

```
EX-1 (Inline Eval) ──┬──► EX-2 (Verification) ──► EX-3 (Constraint Monitor)
                      │
                      └──► EX-4 (State Machine) ──► EX-5 (Action Trace)
                      │
                      └──► EX-6 (CLI Runner, independent)
                                                    │
                      EX-7 (Analysis) ◄─────────────┘
                           │
                           └──► EX-8 (Trade Studies)
```

EX-1 is the critical first step — it establishes the LSP↔Runner integration pattern that all subsequent phases reuse.

## Beyond EX-8: port flow, ODE, physics

Phases 10–15 extended the runtime with port-aware simulation, parametric solving, plugin solvers, and ODE-with-state-machine coupling. Phases 16+ (Apr 12–14 2026) added the physics-aware layer and the SysML-to-Simulation pipeline. All of it has shipped; the module map above and the `sysml-runtime` source are the current description.

| Phase | Feature | Status |
|-------|---------|--------|
| 10 | Port-aware flow routing (PortInstanceIR, typed endpoints, interface contracts) | **DONE** |
| 11 | Parametric constraint solving (propagation, rollups, numeric, DOF, sensitivity) | **DONE** |
| 12 | Reactive port simulation (TriggerKind::PortMessage, TickStrategy, port events) | **DONE** |
| 13 | Activity token flow with port routing (port_target/source, sequence traces) | **DONE** |
| 14 | Solver plugin API (SolverPlugin trait, SolverRegistry, builtin solvers) | **DONE** |
| 15 | ODE continuous simulation (RK4/RK45, expression-based RHS, signal expressions) | **DONE** |
| 16 | Physics-aware static layer (ISQ inference, classification, domain detection, PH001–PH006 diagnostics) | **DONE** (Apr 13 2026) |
| 17 | Hybrid executor + dynamic topology (port-flow can change mid-simulation; valve gating, three-phase AC, DC motor) | **DONE** (Apr 13 2026) |
| 18 | GatedExpression (SM-state-gated expressions), variable impedance | **DONE** (Apr 13 2026) |
| 19 | SysML-to-Simulation Pipeline (Phase A CSV @DataSource, Phase D @ToolVariable signals, Phase E hybrid-core integration tests) | **DONE** (Apr 14 2026) |

The physics-aware layer in `sysml-core/src/physics/` performs **static analysis** before any simulation: ISQ unit inference on attributes, conservation-law classification (KCL/KVL/Newton/thermal-balance), and domain detection (electrical, thermal, mechanical, fluidic). Runtime re-exports these from `sysml-core` so execution can choose a domain-appropriate solver. The LSP wires the analysis as PH001–PH006 diagnostics with spans, codes, notes, and quick-fix code actions (e.g., "did you mean ISQ::electricalConductance instead of Real?").

### Phase 10: Port-Aware Flow Routing

**New module**: `flows/port.rs` — `PortInstanceIR`, `PortFeature`, `PortDirection`, `PortRegistry`

Bridges sysml-core's port type model (PortDefinition, PortUsage, ConjugatedPortDefinition, FeatureDirectionKind) into the runtime. `compile_ports(graph)` walks PortUsage elements, resolves definitions via `find_feature_type()`, extracts features and direction.

`FlowRouter::route_pending_with_ports(Option<&mut PortRegistry>)` binds delivered payload values to destination port features. Falls back to string-key routing when registry is None (ADR-2).

Port diagnostics FL010-FL015 check type mismatches, missing features, direction conflicts, conjugation issues.

**CLI**: `sysml flow <file> [--inject payload] [--json]`

### Phase 11: Parametric Constraint Solving

**New module**: `solver.rs` — `ConstraintNetwork`, `PropagationResult`, `SolveResult`, `DofAnalysis`, `SensitivityResult`

Four solving strategies stacked by complexity:
1. **Binding propagation**: Extracts equalities from `RelationshipKind::Binding`, propagates known values through chains until fixpoint (always converges)
2. **Rollup patterns**: `compute_rollup(graph, root_id, property, Sum|Max|Min|Avg|Count)` aggregates recursively over part ownership hierarchy
3. **Numeric bisection**: `solve_constraint(expr, known, range)` solves single-unknown constraints via bisection method
4. **Sensitivity analysis**: `sweep_parameter(param, range, steps, constraints, ctx)` finds flip points where constraints change satisfaction

DOF analysis: `analyze_dof(network, constraints)` counts equations vs unknowns → Determined/UnderDetermined/OverDetermined.

**CLI**: `sysml solve <file> [--set k=v] [--rollup property] [--sweep param:lo:hi] [--json]`

### Phase 12: Reactive Port Simulation

**Extended**: `statemachine/mod.rs`, `orchestrator.rs`

`TriggerKind::PortMessage { port_name, payload_type }` — new trigger variant for state machine transitions fired by port message delivery.

`TickStrategy` enum (ADR-4):
- `StepFirst` (default): step subsystems → route messages (original behavior)
- `RouteFirst`: route messages → deliver to ports → generate trigger events → step subsystems

The orchestrator converts FlowRouter deliveries to port events, which become the effective event string for state machine stepping. Payload captured in EvalContext as `portName_payload`.

### Phase 13: Activity Token Flow with Port Routing

**Extended**: `actions/mod.rs` — `port_target: Option<String>` on Send nodes, `port_source: Option<String>` on Accept nodes. When set, messages route through FlowRouter to specific ports.

**New module**: `sequence.rs` — `SequenceTrace`, `SequenceTraceBuilder`

Records simulation events as an interaction model with lifelines (for parts) and messages (for flows). `SequenceTraceBuilder` deduplicates lifelines and maintains monotonic sequence numbers. `trace_from_snapshots()` builds traces from orchestrator execution history.

**CLI**: `sysml trace <file> [--inject source:payload] [--json]`

### Key Spec Foundations (SysML v2 / KerML)

- **Flows**: `FlowUsage` is both Action AND Connector — can participate in succession chains and state transitions
- **Ports**: `Port::outgoingTransfersFromSelf` must target ports connected by an Interface (spec-enforced)
- **Transfers**: `MessageTransfer` (trigger-based) is disjoint from `FlowTransfer` (dataflow)
- **Triggers**: `TransitionPerformance::trigger` is `MessageTransfer[*]` — port flows trigger state transitions
- **State dispatch**: `acceptable`/`accepted`/`deferrable` transfer semantics with `incomingTransferSort`
- **Temporal**: `HappensBefore`, `HappensDuring`, `HappensWhile` — axiomatic ordering model
- **Clocks**: `universalClock`, `TimeOf(occurrence, clock)`, `TriggerAt`/`TriggerAfter`
- **Gap**: No operational execution semantics (only denotational). We define the execution model.

### Architectural Decisions (ADRs)

| ADR | Decision |
|-----|----------|
| ADR-1 | PortInstanceIR lives in sysml-runtime, populated from ModelGraph |
| ADR-2 | PortRegistry is optional companion to FlowRouter (None = string fallback) |
| ADR-3 | Binding propagation before numeric solving |
| ADR-4 | Tick loop reorder via TickStrategy enum (StepFirst default) |
| ADR-5 | Sequence trace as first-class output format |
| ADR-6 | No external dependencies until Phase 14 (plugin API) |

See `crates/lang/sysml-runtime/src/flows/` for the implementation.

### Phase 14: Solver Plugin API

**New modules**: `solver_plugin.rs`, `solver_registry.rs`, `solver_builtins.rs`

`SolverPlugin` trait defines the interface for external/builtin solvers: `capabilities()`, `solve()`, `name()`. `SolverRegistry` discovers and dispatches to plugins. Built-in solvers (propagation, bisection, ODE) register via the plugin API at startup.

`SolverCapabilities` describes what a solver can do (continuous, discrete, algebraic, optimization). `SolverParam` and `SolverResult` provide type-safe parameter passing and result extraction.

### Phase 15: ODE Continuous Simulation

**New modules**: `ode.rs`, `ode45.rs`, `ode_builder.rs`, `ode_events.rs`, `clock.rs`

This is the most significant execution feature — continuous-time ODE simulation coupled with discrete state machines via the orchestrator.

#### ODE Solvers

Two solvers available:

| Solver | Module | Method | Use Case |
|--------|--------|--------|----------|
| `Rk4Solver` | `ode.rs` | Fixed-step RK4 | Predictable timing, simple dynamics |
| `Rk45Solver` | `ode45.rs` | Adaptive RK4/5 (Dormand-Prince) | Stiff systems, variable step size |

Both implement the same interface: `step(t, dt, &EvalContext) → ()` and `sync_to_context(&mut EvalContext)`.

#### ODE Builder — Expression-Based RHS

`ode_builder.rs` bridges SysML constraint expressions to ODE right-hand sides:

```rust
let expr = parse_derivative("(heaterPower - lossCoeff * (T - Tamb)) / thermalMass")?;

let spec = OdeSpec::new()
    .with_state_var("temperature", 298.15, expr)
    .with_param("heaterPower", 100.0)
    .with_param("thermalMass", 5.0)
    .with_signal("heaterPower", parse_derivative("100.0 * sin(2.0 * 3.14159 * t)")?)
    .build_solver("thermal-ode");
```

The `OdeRhs` closure receives `(t, y, &EvalContext)` and evaluates each derivative expression with the current state bound. Evaluation order per tick:

1. Template parameters (defaults from SysML model)
2. Orchestrator context overlay (runtime parameter injections from UI)
3. Time `t` and state variables `y[i]`
4. **Signal expressions** (time-varying inputs, evaluated before derivatives)
5. Derivative expressions (produce `dy/dt`)

#### Signal Expressions

Parameters can be time-varying via `@ToolVariable { signal = "expr(t)"; }`:

```sysml
attribute loadCurrent : Real = 48.0 {
    @ToolVariable { name = "loadCurrent";
        signal = "48.0 + 10.0 * sin(2.0 * 3.14159 * 50.0 * t)"; }
}
```

The signal expression is evaluated each ODE tick with the current simulation time `t`. Available functions: `sin()`, `cos()`, `abs()`, `max()`, `min()`, `sqrt()`, plus all standard expression operators. The signal value overrides the parameter's static default and is visible in the UI.

Signal values are synced back to the orchestrator context after each step for display purposes (the ODE RHS evaluates them internally, but the context needs updating so the step response includes live values).

#### Orchestrator: Coupling ODE with State Machines

The `Orchestrator` (`orchestrator.rs`) coordinates multiple subsystems in a tick loop:

```
┌─────────── Orchestrator Tick ───────────┐
│                                          │
│  1. ODE subsystems step (dt seconds)     │
│     └─ sync_to_context() updates vars    │
│                                          │
│  2. Context merge: ODE values → SM       │
│     └─ bimetalTemp, faultIntegral, etc.  │
│                                          │
│  3. State machine step                   │
│     └─ Guards evaluated with ODE values  │
│     └─ Guard-only transitions auto-fire  │
│                                          │
│  4. Context merge: SM outputs → ODE      │
│     └─ Entry actions can set parameters  │
│                                          │
│  5. Snapshot captured                    │
└──────────────────────────────────────────┘
```

**Guard-only transitions**: `accept when bimetalTemp >= 423.15` creates a transition with no explicit trigger event. The state machine compiler reads the `trigger` property from `TransitionUsage` elements, strips the `when ` prefix, and compiles it as a guard expression. Each tick, if no explicit event fires, guard-only transitions are evaluated and auto-fire when their condition becomes true. This is the mechanism for ODE threshold crossings to trigger state changes.

#### Spec-Aligned Metadata: ToolExecution + ToolVariable

The SysML v2 spec defines `ToolExecution` and `ToolVariable` metadata in `AnalysisTooling.sysml`. Our implementation follows this pattern exactly:

```sysml
part def ThermalModel {
    // Annotate the part with the solver to use
    metadata ToolExecution { toolName = "builtin:ode-rk4"; }

    // State variables: direction=out, with derivative expression
    out attribute temperature : Real = 298.15 {
        @ToolVariable { name = "temperature";
            derivative = "(heaterPower - loss * (temperature - ambient)) / mass"; }
    }

    // Parameters: no direction (default=in), optionally with signal
    attribute heaterPower : Real = 100.0 {
        @ToolVariable { name = "heaterPower";
            signal = "100.0 * sin(6.283 * t)"; }
    }
    attribute ambient : Real = 298.15;
    attribute mass : Real = 5.0;
    attribute loss : Real = 10.0;
}
```

**Auto-discovery pipeline** (`detect_ode_from_metadata` in sysml-service):

1. Scan graph for `MetadataUsage` typed as `ToolExecution` with `toolName = "builtin:ode-rk4"` (or `rk45`)
2. Find the annotated element (part def) via `metadata.owner`
3. `get_tool_variables()` iterates the element's `AttributeUsage` children, finds `@ToolVariable` metadata on each
4. Classify by direction: `out`/`inout` → state variable (extract `derivative`), else → parameter (extract `signal`)
5. Compile derivative and signal expression strings to `ExprIR` via `parse_derivative()`
6. Build `OdeSpec` → `Rk4Solver` or `Rk45Solver`
7. Add to `Orchestrator` alongside the state machine

**Metadata body member parsing note**: Inside `@ToolVariable { name = "x"; derivative = "expr"; }`, the body members parse as `DefaultReferenceUsage` (not `AttributeUsage`) via the PEG grammar's ordered choice. The metadata query function `extract_string_attr()` handles both `AttributeUsage` and `ReferenceUsage` children.

#### Runtime Parameter Injection

ODE parameters can be changed at runtime via the `orchestrator_step` API:

```json
POST /sessions/orchestrator/{key}/step
{
    "event": null,
    "parameter_overrides": [["loadCurrent", "20.0"], ["branch flow", "0.05"]]
}
```

Overrides are injected into the orchestrator's `EvalContext` before stepping. The ODE RHS reads parameters from the context (via `merge_from(ctx)`), so overrides take effect on the next tick. The sim app's `EditableVariable` component queues overrides that are drained and sent with each step.

**Precedence** (lowest to highest): template defaults → orchestrator context → signal expressions → state variable bindings.

#### End-to-End Flow: Simulation App

```
┌─ Sim App (React) ─────────────────────────────────────┐
│  SimulateMode → useAdaptiveSimulation → api.client     │
│  ├─ detectSessionType(caps) → 'orchestrator'           │
│  ├─ continuousAutoStart(uri, smName, dtMs)             │
│  ├─ orchestratorStep(key, event?, paramOverrides?)     │
│  └─ EditableVariable.commit() → queueOverride()       │
└────────────────────────────────────────────────────────┘
         │ HTTP via Vite proxy (:3010 → :8080)
         ▼
┌─ sysml-api (axum) ────────────────────────────────────┐
│  /api/command → dispatch_command()                      │
│  /sessions/orchestrator/:key/step → orchestrator_step() │
└────────────────────────────────────────────────────────┘
         │
         ▼
┌─ sysml-service ───────────────────────────────────────┐
│  continuous_auto() → detect_ode_from_metadata()        │
│  ├─ Finds @ToolExecution, extracts vars/params/signals │
│  ├─ Compiles derivatives + signals to ExprIR           │
│  └─ Builds OdeSpec → Rk4Solver → Orchestrator         │
│                                                         │
│  orchestrator_step() → inject overrides → step()       │
│  └─ Sync signal values to context for display          │
└────────────────────────────────────────────────────────┘
         │
         ▼
┌─ sysml-runtime ───────────────────────────────────────┐
│  Orchestrator::step()                                   │
│  ├─ ODE RHS: signals → state update → derivatives      │
│  ├─ sync_to_context() → SM gets ODE values             │
│  ├─ SM step: guard-only transitions check thresholds   │
│  └─ Snapshot: subsystem states + context values        │
└────────────────────────────────────────────────────────┘
```

#### Simulation App UI Controls

| Control | Where | Description |
|---------|-------|-------------|
| **dt** (ms) | Toolbar, pre-start | ODE step size. Smaller = more accurate, more steps. 1ms default. |
| **steps/s** | Toolbar, editable mid-sim | Auto-play rate. setTimeout chaining reads fresh value each tick. |
| **Playback speed** | Toolbar indicator | `dt × steps/s / 1000` = sim-seconds per real-second. |
| **Step** | Toolbar button | Single orchestrator tick. |
| **Auto** | Toolbar button | Continuous stepping at `steps/s` rate. |
| **EditableVariable** | Right panel | Click any variable value to edit. Queued as parameter override. |

#### Expression Standard Library (ODE-relevant)

| Function | Signature | Example |
|----------|-----------|---------|
| `sin(x)` | `Real → Real` | `sin(2.0 * 3.14159 * 50.0 * t)` |
| `cos(x)` | `Real → Real` | `cos(t)` |
| `abs(x)` | `Real → Real` | `abs(loadCurrent)` |
| `sqrt(x)` | `Real → Real` | `sqrt(x ** 2 + y ** 2)` |
| `max(a, b)` | `Real × Real → Real` | `max(0.0, temperature - threshold)` |
| `min(a, b)` | `Real × Real → Real` | `min(voltage, maxVoltage)` |
| `**` | exponentiation | `(current / rated) ** 2` |

All standard arithmetic (`+`, `-`, `*`, `/`), comparison (`<`, `<=`, `>`, `>=`, `==`, `!=`), and logical (`and`, `or`, `not`) operators are available in derivative and signal expressions.
