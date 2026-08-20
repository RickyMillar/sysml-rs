# espresso-pump-hybrid

A compact, analytically understandable **hybrid** model of a reciprocating
positive-displacement pump. It exercises, in one small fixture:

- **nonlinear sampled data** — two CSV-backed `SampledFunction` check-valve
  characteristics, selected by command direction (hysteresis branches);
- **oscillatory ODE behaviour** — an undamped actuator plus two coupled
  pressure states integrated by the spec `StateSpaceRepresentation` pattern;
- **event location** — a state-machine cycle driven by located zero-crossings
  of the actuator state;
- **state-machine coordination** — a five-phase reciprocating cycle with a
  latched safety path;
- **model-level accumulation** — an exposure integral of sustained
  over-pressure;
- **simulation-backed verdicts** — verification cases with analytically
  justified pass/fail bounds.

Everything here is derived from generic first principles and the programme's
capability contract. No measured device curve, product model, calibration
value, threshold, or waveform is used. The CSV tables are emitted by a
deterministic generator (`scripts/generate_characteristics.py`) whose exact
bytes, equations, and SHA-256s are pinned in `fixture-provenance.toml`.

## Files

```
Libraries/Tooling.sysml        @DataSource metadata definition
Libraries/Types.sysml          hydraulic effort/flow payload (PressureValue + VolumeFlowRateValue)
Libraries/Interfaces.sysml     fluid-power port/part vocabulary (structural view, not simulated)
Physics/PumpCharacteristic.sysml   the two @DataSource-backed SampledFunction branches
Physics/PumpODE.sysml          ReciprocatingPump part: states, parameters, continuous dynamics
Physics/HydraulicConstraints.sysml static well-formedness constraints
Behaviour/PumpCycle.sysml      the reciprocating-cycle + latched-relief state machine
Scenarios/Restrictions.sysml   the nominal / moderate / severe restriction levels
Verification/PumpSafety.sysml  requirement + verification-case definitions
data/generated_pump_*.csv      generated check-valve characteristics
scripts/generate_characteristics.py  deterministic generator
fixture-provenance.toml        provenance manifest (SHA-256 + equations + regen command)
```

## State, interface, and parameter contract

| Symbol | Model name | Meaning | Units | Role | Default |
|---|---|---|---|---|---|
| `x` (centered) | `stroke` | actuator position about mid-stroke; physical position = `stroke + 0.5` in [0,1] | normalized | ODE state | -0.5 |
| `v` | `velocity` | actuator velocity | normalized/time | ODE state | 0.0 |
| `p_c` | `chamberPressure` | chamber pressure | pressure | ODE state | 0.0 |
| `p_a` | `accumulatorPressure` | downstream/accumulator pressure | pressure | ODE state | 0.0 |
| `z` | `exposure` | integral of sustained over-pressure | pressure·time | ODE state | 0.0 |
| `ω` | `omega` | actuator angular frequency | rad/time | parameter | 6.2832 |
| `S_d` | `pistonArea` | displacement gain on the advancing stroke | area | parameter | 0.5 |
| `C_q` | `dischargeCoeff` | orifice discharge coefficient | — | parameter | 2.0 |
| `Δp_ref` | `dpRef` | command-saturation pressure difference | pressure | parameter | 1.0 |
| `ε` | `epsRoot` | square-root regularization floor | pressure | parameter | 1e-4 |
| `k_L` | `leakCoeff` | linear chamber leak | 1/time | parameter | 0.05 |
| `K_c` | `chamberGain` | chamber compliance factor | — | parameter | 4.0 |
| `K_a` | `accumulatorGain` | accumulator compliance factor | — | parameter | 2.0 |
| `G` | `restrictionConductance` | downstream discharge conductance (scenario) | — | parameter | 1.5 |
| `p_w` | `pWarning` | accumulator over-pressure threshold | pressure | parameter | 0.5 |
| `z_trip` | `exposureTrip` | latched-relief exposure threshold | pressure·time | parameter | 1.0 |
| `u` | `u` (signal) | normalized valve command | — | branch abscissa | — |
| `A_eff` | `aEff` (signal) | effective valve area | normalized area | sampled output | — |

## Equations (independently derived)

**Actuator** — an undamped harmonic oscillator about mid-stroke, so it
self-sustains a phase-normalized cosine with no manual seed:

```
d(stroke)/dt   = velocity
d(velocity)/dt = -ω² · stroke
```

Released from `stroke = -0.5, velocity = 0`, this gives
`x(t) = 0.5·(1 - cos ω t)` (physical position sweeping 0 → 1 → 0 each cycle).
Because it is undamped, its mechanical energy `E = ½v² + ½ω²·stroke²` is
conserved — the bounded-drift / conservation invariant the ODE gate checks.

**Hydraulics** — a forward-only check valve feeding an accumulator that
discharges through a restriction:

```
q_src  = S_d · max(0, velocity)                      delivery-stroke inflow
Δp     = chamberPressure − accumulatorPressure
u      = clamp(Δp / Δp_ref, 0, 1)                    normalized valve command
A_eff  = interpolate( velocity ≥ 0 ? opening : closing branch, u )
q_out  = C_q · A_eff · sqrt( max(Δp, ε) )            regularized orifice outflow
q_leak = k_L · chamberPressure
q_rest = G · sqrt( max(accumulatorPressure, ε) )
d(chamberPressure)/dt     = K_c · (q_src − q_out − q_leak)
d(accumulatorPressure)/dt = K_a · (q_out − q_rest)
d(exposure)/dt            = max(0, accumulatorPressure − p_w)
```

Design choices worth stating:

- **`ε` regularization.** `sqrt(max(·, 0))` has infinite slope at zero, which
  is ill-conditioned for the solver's Jacobian. `sqrt(max(·, ε))` bounds the
  slope to `1/(2·√ε)` and keeps the derivative continuous. The check valve is
  **forward-only** (no signed reverse branch): the sampled area collapses to
  `A_min` as `u → 0`, so residual reverse flow is negligible rather than modeled
  with a discontinuous `sign`.
- **Branch selection.** `velocity ≥ 0` (advancing stroke, rising command) uses
  the *opening* branch; a retracting stroke uses the *closing* branch.
  `velocity ≥ 0` is the **explicit initial branch** (velocity starts at 0).
- **Exposure on the accumulator.** The chamber pressure pulses every stroke; the
  accumulator smooths it, so the physically-meaningful sustained-hazard signal
  is the downstream pressure. Keying the exposure integral on
  `accumulatorPressure` avoids per-stroke false trips.

## Check-valve characteristics (the generated tables)

`scripts/generate_characteristics.py` emits both branches over a 64-point
command grid `u_i = i/63`, from closed forms:

```
S(u)       = u²·(3 − 2u)                    smoothstep, monotone on [0,1]
opening(u) = A_min + (A_max − A_min)·S(u)    A_min = 0.02, A_max = 1.0
sep(u)     = H·u²·(1 − u)²                   bounded, vanishes at u = 0 and u = 1
closing(u) = opening(u) − sep(u)             H = 0.64
```

Both branches are monotone non-decreasing (the closing branch stays monotone
because `H = 0.64 ≤ 3·(A_max − A_min) = 2.94`), pinned to `A_min` at `u = 0` and
`A_max` at `u = 1`, and the hysteresis separation peaks at `H/16 = 0.04` at
`u = 0.5` and vanishes at both endpoints. The generator asserts every one of
these properties before writing and aborts nonzero on any violation.

## Cycle state machine

```
Idle → Intake → Compress → Discharge → Recover → Intake → …
                                             \→ Relieved (latched)
```

The four cycle transitions are located zero-crossings of the actuator state:

| transition | located crossing | actuator instant |
|---|---|---|
| Intake → Compress | `stroke > 0` (rising) | quarter cycle, position at mid |
| Compress → Discharge | `velocity < 0` | half cycle, position at max |
| Discharge → Recover | `stroke < 0` (falling) | three-quarter cycle, position at mid |
| Recover → Intake | `velocity > 0` | full cycle, position at min |

`Idle → Intake` is the explicit initial branch. The latched safety path fires
from any cycle state when `exposure > exposureTrip`; `Relieved` is terminal, so
the first-activation time is recorded once and never overwritten. The exposure
integral is itself the anti-chatter/debounce: a brief pressure transient cannot
accumulate enough exposure to trip.

## Restriction scenarios and the verification bounds

The scenario knob is the discharge conductance `G` (`restrictionConductance`).
At cycle-average steady state, inflow balances outflow, so the accumulator
settles near `p_a ≈ (q̄_src / G)²` with `q̄_src = S_d · avg(max(0, velocity))`.
Larger `G` (free discharge) settles the accumulator **below** `p_w`; smaller `G`
(restricted) settles it **above** `p_w`, which is what drives the exposure
integral.

| scenario | `G` | accumulator vs `p_w = 0.5` | relief |
|---|---|---|---|
| nominal | 1.5 | settles below (measured `p_a,max ≈ 0.30`) | never |
| moderate | 0.6 | settles near | marginal |
| severe | 0.3 | settles above (measured `p_a,max ≈ 1.62`) | latches |

**Verification requirements** (`Verification/PumpSafety.sysml`) read the
simulation-produced state at the horizon:

- `SafeUnderNominal`: `exposure < exposureTrip`. Nominal PASSES (exposure ≈ 0);
  severe FAILS.
- `ProtectsUnderSevere`: `exposure ≥ exposureTrip`. Severe PASSES; nominal FAILS.
- `BoundedChamberPressure`: `chamberPressure² ≤ 100`. PASSES for every scenario
  (numerical-stability envelope).

The first two are complementary, so the scenario × requirement verdict matrix is
non-vacuous (diagonal pass, off-diagonal fail).

**Analytic bounds asserted by the gates:**

- *No nominal relief (PUMP-SAFE-01).* Nominal `p_a,max < p_w`, so
  `d(exposure)/dt = max(0, p_a − p_w) = 0` throughout and `exposure` never
  reaches `exposureTrip`.
- *Severe dwell lower bound (PUMP-SAFE-02, anti-chatter).* Since
  `d(exposure)/dt ≤ p_a,max − p_w`, we have
  `exposure(t) ≤ (p_a,max − p_w)·t`, so relief (at `exposure = exposureTrip`)
  **cannot** occur before
  `t_lo = exposureTrip / (p_a,max − p_w)`.
  With the measured severe `p_a,max ≈ 1.62`, `t_lo ≈ 1.0 / 1.12 ≈ 0.89 s`; the
  located relief occurs at ≈ 3.82 s — after the dwell and inside the horizon.
- *Event-time convergence (PUMP-HYB-02).* The located relief time is
  dt-independent in the resolved regime: it moves by well under one coarse step
  as `dt` halves (3820 ms at `dt = 4 ms` and `dt = 2 ms` → 3819 ms at `dt = 1 ms`
  and `dt = 0.5 ms`).

> The three measured constants above (severe `p_a,max`, the located relief time,
> and its dt series) were corrected on 2026-08-19 from `≈ 2.2`, `≈ 3.19 s` and
> `≈ 3188 ms`. Those figures had drifted away from the model: every gate in
> `crates/lang/sysml-runtime/tests/espresso_pump_hybrid.rs` derives its bound
> from the run it just made (`t_lo` is computed from that run's own `p_a,max`),
> so nothing ever compared the prose against the fixture. The safety argument is
> unchanged — relief still falls well after the dwell bound — but the numbers
> now match what the model produces, and `pump_safe_03_readme_measured_constants_still_hold`
> fails if either side drifts again.

## Running it

```bash
# regenerate the characteristic tables (deterministic, byte-stable)
cd examples/espresso-pump-hybrid && python3 scripts/generate_characteristics.py

# the runtime + generator gates
cargo test -p sysml-runtime  --test espresso_pump_hybrid
cargo test -p sysml-spec-tests --test espresso_pump_generator
cargo test -p sysml-spec-tests --test fixture_provenance
```
