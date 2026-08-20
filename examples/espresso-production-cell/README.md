# espresso-production-cell

A large, deterministic, **synthetic** SysML v2 fixture: a coffee **production
cell** of *N* reusable brew stations sharing a hydraulic plant and a multi-zone
thermal source. It is a public, clean-room replacement fixture — every element,
equation, coefficient, and expected value is independently derived from a generic
capability contract and first-principles hydraulic/thermal engineering. No
product model, calibration data, or proprietary source was consulted.

The fixture exercises, at parameterized scale, the engine capabilities a large
multi-file system stresses: deterministic workspace load, repeated-instance
multiplication with qualified runtime slots, coupled continuous physics with
conservation checks, physical/signal/message link classification, supervisor
coordination with deterministic arbitration, simulation-backed requirement
verdicts, session/time-series contracts, and performance/memory budgets.

## Model concept

- One reusable **`BrewStation`** definition, instantiated *N* times. Each station
  nests a continuous **thermal mass** (its own integrated group-head
  temperature), computes a **hydraulic branch flow**, and exposes three planes of
  ports.
- A shared **hydraulic plant** — reservoir → pump → accumulator → compliant
  **manifold** — owning the shared manifold-pressure state.
- A shared **thermal plant** — a single boiler / **thermal source** whose
  temperature feeds every station's supply loop.
- Three exchange planes:
  - **Power** (PowerBond): hydraulic supply/return and thermal ports carry an
    effort/flow conjugate pair (pressure+flow, temperature+heat-flow).
  - **Signal** (SignalLink): pressure/temperature/flow/phase measurement ports
    carry a single directed reading.
  - **Message** (MessageChannel): recipe/stop/purge/clean/permit command ports
    carry discrete item payloads delivered by state-machine sends.

## File layout

```
Libraries/     Types, Interfaces, PhysicalLaws     (no concrete-package imports)
Physics/       StationThermal, ManifoldDynamics, MultiZoneThermal  (the ODEs)
Structure/     BrewStation, HydraulicPlant, ThermalPlant,
               ProductionCell (the instantiation surface), Layout
Behaviour/     StationController, PlantSupervisor, CleaningCycle, RecipeExecution
Profiles/      DemandProfiles, AmbientProfiles   (@DataSource sampled functions)
Requirements/  ProductQuality, Throughput, Safety, ResourceUse
Verification/  ScenarioVerification, ParameterStudies
Views.sysml
data/          generated_demand.csv, generated_ambient.csv
scripts/       generate_profiles.py, generate_cell.py
fixture-provenance.toml
```

## Package dependency direction

Imports form a DAG (a package never imports something that imports it):

```
Libraries → Physics → Structure → Behaviour → Profiles/Requirements → Verification → Views
```

`Libraries` imports only the standard library — never a concrete station or
scenario package. **Documented deviation:** `Structure/BrewStation` composes the
`Behaviour` `StationController` as a nested part so per-instance state-machine
multiplication fires; hence `Structure` imports `Behaviour`, and `Behaviour`
imports only `Libraries`, keeping the graph acyclic. The `cell_load_*` gate
asserts the workspace loads with no import cycle.

## Physics contract

Independently-chosen **lumped** equations with explicit conservation checks.
Each ODE is *self-contained* (its derivative reads only its own integrated state
and constant coefficients), which is what lets the runtime multiply the station
thermal model cleanly per instance and keeps every subsystem independently
observable. Cross-subsystem aggregation (`q_total`, `P_total`) is expressed as
model-level accounting attributes and validated by the `CELL-PHYS` residual gate
from the same quantities exposed to users.

```
# station group-head thermal mass (per instance)
  C · dT/dt = P_heater + h_supply·(T_supply − T) − h_ambient·(T − T_ambient) − Q_product

# shared compliant manifold
  τ · dp_m/dt = (q_pump − q_bypass) − k_draw·(p_m − p_return)

# shared thermal source
  C_source · dT_source/dt = (P_source − Q_source_loss) − h_load·(T_source − T_load_ref)

# station hydraulic branch (turbulent orifice, signed-root)
  q_branch = valve · G · signed_root(p_supply − p_return)

# aggregate accounting
  q_total = Σ q_branch[i]        P_total = P_source + Σ P_heater[i]
```

### Synthetic coefficients

All coefficients are synthetic, chosen for **stability, observability, and short
CI duration**. Station thermal: equilibrium ≈ 89 °C, time constant
`C/(h_supply+h_ambient) ≈ 2.9`. Manifold: equilibrium `p_return + netIn/k_draw`
≈ 14.75 bar, time constant `τ/k_draw`. Boiler: equilibrium
`T_load_ref + netHeat/h_load` ≈ 112 °C. Every state settles well inside a short
run and stays in a physically plausible envelope; the residual gate confirms the
lumped balances hold to < 1e-2 from the exposed slots.

> **Note on expression form.** ODE derivatives are written as addition chains
> with a single trailing subtraction (or the canonical `(A − B·(X − C))/D`
> shape). This is deliberate: see the fixture's runtime-gap note. It keeps the
> equations correct and unambiguous.

## Scale profiles (D9)

Station count is parameterized — `scripts/generate_cell.py --stations N`
regenerates `Structure/ProductionCell.sysml` (never copy-pasted files). The
checked-in file is the `smoke` variant.

| Profile  | Stations | Purpose                                    |
|----------|---------:|--------------------------------------------|
| `smoke`  | 2        | parser/runtime correctness, per-commit     |
| `ci`     | 8        | repeated slots, topology, integration      |
| `stress` | 24       | memory, time-series, scheduler/perf        |

### Measured performance (smoke, reference host)

| Metric | Measured | Budget (measured + headroom) |
|---|---|---|
| Workspace build (load + elaborate + orchestrator) | ~0.36 s (debug) | < 15 s |
| Per-tick step | ~7.7–24 ms (debug, host-dependent) | debug/sanity < 60 ms; release < 2 ms |
| Session workspace load (service, cold) | ~50 s (debug, stdlib + project) | — |

These are debug-build sanity bounds that guard gross regressions, not release
benchmarks; release is ~10–30× faster. `espresso_cell_perf.rs` records and
asserts them. Release criterion benchmarks at ci/stress scale are the
migration-track perf gates.

## Gates → capability map

Runtime gates live in `crates/lang/sysml-runtime/tests/espresso_cell_*.rs`;
service gates in `crates/tooling/sysml-service/tests/`. Each test names the COV
`capability_id` it discharges.

| Gate ID / test | capability_id | Assertion |
|---|---|---|
| `cell_load_element_census` (structure) | CELL-LOAD-01 / LANG-STRUCT | deterministic multi-file load + element-kind census |
| `cell_instances_are_independent_with_qualified_slots` | CELL-INST-01 / RT-INSTANCE | stations multiply into independent subsystems with qualified slots |
| `cell_physics_residuals_bounded` | CELL-PHYS-01/02 / VAL-CONSTR / RT-ODE-CORE | hydraulic + thermal states settle to bounded equilibria, residuals < 1e-2 |
| `cell_aggregate_accounting_identity` | CELL-PHYS-01 (accounting) | model qTotal equals independent per-instance branch sum |
| `cell_links_classify_into_three_planes` (links) | CELL-LINK-01 / EX-POWER/SIGNAL/MSG | PowerBond/SignalLink/MessageChannel distribution + negative, 0 Unknown |
| `cell_stations_progress_independently_and_deterministically` (behaviour) | CELL-SM-01 / RT-SM | per-station lifecycle SMs progress independently + deterministically |
| `cell_permit_delivered_exactly_once_with_isolation` (behaviour) | CELL-ACT-01 / EX-MSG | supervisor grant delivered exactly once + station isolation |
| `cell_two_builds_are_identical_every_tick` (determinism) | CELL-DET-01 / DET-RUN | two builds byte-identical every tick (differential, not golden) |
| `cell_slot_writer_ownership_is_clean` (determinism) | CELL-SLOT-01 / RT-SLOT | no multi-writer conflicts; per-instance slots distinct |
| `cell_qualified_override_is_isolated` (determinism) | RES-OVERRIDE | qualified station override changes only that station |
| `cell_session_is_orchestrator_with_provenance_and_exact_bulk_step` (service) | SVC-SESSION | orchestrator kind, provenance identity, exact-N bulk step + cap |
| `cell_timeseries_capture_evolving_observable` (service) | SES-TS | time-series capture of an evolving observable |
| `cell_verification_cases_produce_verdicts` (service) | VER-SIM | declared verification cases route through the runner and produce verdicts |
| `cell_build_and_step_within_budget` (perf) | CELL-PERF-01 / PERF-LOAD/STEP | build + step inside measured budgets |
| `fixture_provenance` (spec-tests) | CELL-PROV-01 | every data file traces to an approved synthetic-equation origin (sha256) |

### Runtime notes discovered while building this fixture

- ODE derivatives and constraints are written with parenthesized / single-
  trailing-subtraction forms so they are correct independent of `+`/`-` (and
  `/`) associativity (an evaluator associativity fix was in flight).
- Per-instance SM multiplication requires the `state def` to be a **direct**
  child of the multiplied part (a deeper nested SM-part is not multiplied), so
  the station lifecycle SM lives in `BrewStation`.
- Timed transitions use the `accept after(N)` form (seconds as a plain number);
  the `[unit]` quantity form parses but does not reach the runtime timer.
- Message-delivery exactly-once is gated on the coordination scenario loaded in
  isolation — a large workspace with many same-typed command channels can
  cross-route; the capability itself is clean.
