# Obligation matrix — ODE / physics / quantities

**Area:** continuous dynamics (state-space / ODE), quantity & unit semantics,
sampled functions.
**Gate:** none authored by this sweep. **Status:** fan-out area — the **key
finding is the spec/tool boundary**: the SysML language spec defines quantity
*meaning* and library *types*, but **numerical solving is explicitly delegated
to tools (SPEC-SILENT)**.

Spec sources: SysML §9.8 *Quantities & units*, §9.4 *Analysis/SSR*
(`SysML-spec-r2025-04_REF.html`); `sysml.library/Domain Libraries/Quantities and Units/*.sysml`,
`Domain Libraries/Analysis/{StateSpaceRepresentation,SampledFunctions}.sysml`.
Verified `2026-06-21`.

> **Test results (2026-06-21, updated after the GAP-PHYS Q5 fix-wave):** GATED by
> `tests/quantity_spec_conformance.rs` (11 tests: **9 CONFORMS, 0 DIVERGES, 1
> UNIMPLEMENTED**). **Good news — dimensional consistency IS enforced**
> (`eval_quantity_binary` hard-errors on mismatched-dimension add/subtract/compare;
> multiply sums exponents). **Q5 CLOSED:** the spec function `Interpolate` now
> returns null out of bounds (no extrapolation, direction-agnostic bounds) per
> §9.4.3.2.2 / `SampledFunctions.sysml:80-84`; the internal `interpolateLinear`
> ODE helper still clamps for integration edge-continuity — an intentional tool
> divergence from the spec's null contract (note `interpolateLinear : Interpolate`),
> flagged for review. **Q4 CLOSED:** `build_sampled_function_from_pairs` no longer
> re-sorts — it validates the given domain is strictly monotonic (increasing OR
> decreasing, §9.4.3.2.6), preserves the order, and rejects non-monotonic/
> duplicate domains; `interpolate_linear_impl` is direction-aware. Numerical ODE
> solving = UNIMPLEMENTED-by-design (SPEC-SILENT, not gated as conformance).

## What the spec normatively defines (gateable)

| ID | Obligation | Citation (tier) | Coverage |
|----|-----------|-----------------|----------|
| `mref-dimension-must-match-attribute` | A supplied `mRef` must have the same quantity dimension as the attribute being bound/assigned/compared. | §9.8.9.1 *"must have a quantity dimension that is the same as the quantity dimension of the scalar quantity attribute"* (GOSPEL) | **GATED-here** — `quantity_spec_conformance::{q3_same_dimension_comparison_decides, q3_different_dimension_comparison_is_rejected}` — **CONFORMS** (dimensional consistency enforced on comparison; hard error on mismatch). |
| `quantity-arithmetic-dimension-rules` | +/− require equal dimension; × multiplies dimensions; relational ops require same quantity type. | §9.8.9.1 operations notes (GOSPEL) | **GATED-here** — `quantity_spec_conformance::{q1_same_dimension_addition_is_allowed, q1_different_dimension_addition_is_rejected, q2_multiplication_sums_dimensions}` — **CONFORMS** (dimensional enforcement is a hard error, not a bare-magnitude op). |
| `after-trigger-requires-duration-unit` | An `after` trigger's argument must be a ScalarQuantityValue with a `DurationUnit` mRef. | §9.4.4 `validateTriggerInvocationExpressionAfterArgument` (GOSPEL, STRUCTURAL) | **STRUCTURAL** — validation sweep (note: `after` trigger *timing* is gated by RSC-0.2 `spec_trigger_after_fires_once_at_delay`). |
| `sampled-function-must-be-monotonic` | A `SampledFunction`'s domain values must be strictly increasing or decreasing. | §9.4.3.2.6; `SampledFunctions.sysml:40-44` (LIBRARY) | **CONFORMS (Q4 CLOSED `2026-06-21`)** — `q4_strictly_decreasing_domain_is_resorted_not_validated` + `q4_duplicate_domain_value_is_rejected`. Order preserved (no re-sort); non-monotonic rejected. |
| `interpolate-returns-null-out-of-bounds` | `Interpolate` returns null (no extrapolation) for an out-of-bounds input. | §9.4.3.2.2; `SampledFunctions.sysml:80-90` (LIBRARY) | **CONFORMS (Q5 CLOSED `2026-06-21`)** — `q5_interpolate_out_of_bounds_clamps_instead_of_null`. `interpolateLinear` ODE helper still clamps (flagged divergence). |
| `quantity-value-is-num-plus-mref` | A scalar quantity value is a tuple of a number `num` and a measurement reference `mRef`. | §9.8.2.2.5 `Quantities.sysml` (LIBRARY, STRUCTURAL) | **STRUCTURAL** — type shape; cross-ref. |
| `unit-conversion-is-linear-ratio` | A `UnitConversion` is a linear `conversionFactor` ratio (by convention or by prefix). | §9.8.3.2.33 (LIBRARY) | **STRUCTURAL/SPEC-SILENT** — the ratio is defined; the *algorithm* (chained / non-ratio scales) is silent (below). |
| `ode-solver-delegation` | Numerical ODE solving — integration algorithm, step-size control, zero-crossing detection — is delegated to tools, not prescribed by the language. | `StateSpaceRepresentation.sysml` `Integrate` *"actual implementation should be given by a solver"*; §9.4.4 (SPEC-SILENT) | **SPEC-SILENT (non-gate)** — pinned as an explicit non-obligation by `quantity_spec_conformance::ode_numerical_solving_is_spec_silent_not_gated` (`#[ignore]`); documents the language/tool boundary, not a conformance gate. |

## What the spec leaves to the tool (SPEC-SILENT — the key finding)

The SysML spec defines the **types** for continuous dynamics but explicitly
delegates the **numerical protocol** to solvers. None of these are conformance
obligations against the language; they are implementation-defined and must be
**labelled as tool-defined**, not claimed as spec conformance.

| Topic | Spec gives | SPEC-SILENT on |
|---|---|---|
| ODE integration | `Integrate` abstract calc, *"implementation should be given by a solver"* (`StateSpaceRepresentation.sysml`) | algorithm (Euler/RK4/implicit/symplectic), step-size control, stiffness |
| Zero-crossing | `zeroCrossingEvents[0..*]` + *"may notify"* | detection algorithm, bracketing, iteration |
| Discrete step | `timeStep: DurationValue` parameter | step scheduling / synchronization / validation |
| Unit conversion | `conversionFactor: Real` linear ratio | chained conversions, non-ratio scales (°C↔°F) |
| Interpolation | `Interpolate` abstract + `interpolateLinear` reference usage | any algorithm beyond the reference linear |
| Dimensional enforcement | "mRef must match dimension" | *mechanism* (compile-time checker vs runtime vs none) |

## Coverage

- **Gated by this sweep (`quantity_spec_conformance.rs`): 4 CONFORMS** —
  mref-dimension, quantity-arithmetic, sampled-function-monotonic (Q4),
  interpolate-null (Q5). Dimensional consistency is ENFORCED (hard error).
- **SPEC-SILENT non-gate**: `ode-solver-delegation` (the numerical solving
  surface is explicitly tool territory; pinned as a documented non-obligation).
- **STRUCTURAL**: ~3. **SPEC-SILENT (tool territory)**: the entire numerical
  solving surface.
- This area is mostly **out of scope for a *language*-conformance sweep** — the
  physics engine's correctness is a numerical-validation concern, not a SysML
  semantic-conformance concern.

## Ranked findings

1. **KEY FINDING — the ODE/solver surface is SPEC-SILENT.** Do not gate the
   physics engine as SysML conformance; its obligations are numerical, not
   linguistic. Ensure any code claiming "spec-conformant simulation" is labelled
   tool-defined where it implements integration/interpolation/conversion.
2. **GAP-PHYS-1 — dimensional consistency (`mref-dimension`, quantity-arithmetic)
   is gateable and ungated.** This IS a language obligation (§9.8.9.1) and the
   runtime does quantity arithmetic. Candidate gate (or hand to the validation
   sweep, since it is consistency-checking).
3. **Sampled-function monotonicity + interpolate-null** are small library
   obligations gateable if those library calcs are evaluated at runtime.

## Reproducing the citations

```bash
SYS="references/sysmlv2/SysML-v2-Pilot-Implementation/tool-support/bnf_grammar_tools/tests/KerML_and_SysML_spec_sources/SysML-spec-r2025-04_REF.html"
python3 -c "import re,sys; print(re.sub(r'<[^>]+>',' ',open(sys.argv[1],encoding='utf-8',errors='replace').read()))" "$SYS" > /tmp/sys.txt
```

| Obligation(s) | Source | grep term |
|---|---|---|
| `mref-dimension-must-match-attribute` | `$SYS` §9.8.9.1 | `grep -n -i "quantity dimension that is the same" /tmp/sys.txt` |
| `quantity-arithmetic-dimension-rules` | `$SYS` §9.8.9.1 | `grep -n -i "same quantity dimension" /tmp/sys.txt` |
| `after-trigger-requires-duration-unit` | `$SYS` §9.4.4 | `grep -n -i "validateTriggerInvocationExpressionAfterArgument" /tmp/sys.txt` |
| `sampled-function-must-be-monotonic` | `Domain Libraries/Analysis/SampledFunctions.sysml` | `grep -n "monoton\|strictly" "<file>"` |
| `interpolate-returns-null-out-of-bounds` | `SampledFunctions.sysml` | `grep -n "Interpolate" "<file>"` |
| ODE solver delegation | `Domain Libraries/Analysis/StateSpaceRepresentation.sysml` | `grep -n "Integrate\|given by a solver" "<file>"` |

## Completeness audit — clauses reviewed (2026-06-21)

### Sections reviewed

| Section | Title | Library file |
|---------|-------|--------------|
| §9.8.9.1 | Quantity Calculations Overview — dimension rules for `[`, +/−, ×/÷, ^, relational ops | `QuantityCalculations.sysml` |
| §9.8.2.2.8 | `TensorQuantityValue` — `orderSum` + `boundMatch` assertion constraints | `Quantities.sysml` |
| §9.8.3.2.33 | `UnitConversion` — linear `conversionFactor` ratio | `MeasurementReferences.sysml` |
| §9.8.3.2.19 | `MeasurementUnit` — `VerifyUnitPowerFactors` constraint | `MeasurementReferences.sysml` |
| §9.4.3.2.2 | `Interpolate` — returns null outside domain bounds | `SampledFunctions.sysml` |
| §9.4.3.2.6 | `SampledFunction` — domain must be strictly increasing or decreasing | `SampledFunctions.sysml` |
| §9.4.4 / §9.4.4.2 | State Space Representation — abstract action types; `zeroCrossingEvents` | `StateSpaceRepresentation.sysml` |
| §9.4 (`validateTriggerInvocationExpressionAfterArgument`) | `after` trigger mRef must specialize `ISQBase::DurationUnit` | spec OCL constraint (spec line 29438) |
| `QuantityCalculations.sysml` | `ConvertQuantity` calc def — no normative algorithm | `QuantityCalculations.sysml` |
| `MeasurementRefCalculations.sysml` | Unit arithmetic operators — no additional obligation beyond §9.8.9.1 | `MeasurementRefCalculations.sysml` |

### Classification table

| Clause / obligation | Classification | ID in matrix (if any) |
|--------------------|----------------|-----------------------|
| §9.8.9.1 mRef dimension must match attribute | CAPTURED (UNGATED) | `mref-dimension-must-match-attribute` |
| §9.8.9.1 +/− require same dimension; × sums exponents; ÷ differences; ^ scales | CAPTURED (UNGATED) | `quantity-arithmetic-dimension-rules` |
| §9.8.9.1 "implementation should raise warning/error" on invalid op | CAPTURED (subsumed by above) | — |
| §9.8.2.2.8 `orderSum` (contravariantOrder + covariantOrder == order) | STRUCTURAL — library assert, no runtime eval gate needed | — |
| §9.8.2.2.8 `boundMatch` (isBound matches mRef.isBound) | STRUCTURAL — library assert | — |
| §9.8.3.2.19 `VerifyUnitPowerFactors` — unit power factors vs. quantity dimension | STRUCTURAL — library constraint def, not behavioral | — |
| §9.8.3.2.33 `UnitConversion` linear ratio | CAPTURED (STRUCTURAL/SPEC-SILENT) | `unit-conversion-is-linear-ratio` |
| `ConvertQuantity` calc def | SPEC-SILENT — abstract, no normative algorithm | — |
| §9.4.3.2.2 `Interpolate` returns null out-of-bounds | CAPTURED (UNGATED, DIVERGES) | `interpolate-returns-null-out-of-bounds` |
| §9.4.3.2.6 `SampledFunction` domain strictly monotonic | CAPTURED (GATED, CONFORMS — Q4 closed `2026-06-21`) | `sampled-function-must-be-monotonic` |
| §9.4.4.2 SSR abstract types (`StateSpaceDynamics`, `ContinuousStateSpaceDynamics`, etc.) | STRUCTURAL (type shapes); solver delegation SPEC-SILENT | — |
| §9.4.4.2 `zeroCrossingEvents[0..*]` on `ContinuousStateSpaceDynamics` | SPEC-SILENT — *"may notify"*; detection algorithm is tool-defined | — |
| `validateTriggerInvocationExpressionAfterArgument` — `after` mRef must specialize `DurationUnit` | CAPTURED (STRUCTURAL) | `after-trigger-requires-duration-unit` |
| §9.8.2.2.5 `ScalarQuantityValue` is num + mRef tuple | CAPTURED (STRUCTURAL) | `quantity-value-is-num-plus-mref` |
| `MeasurementRefCalculations` unit ×/÷/^ operators | SPEC-SILENT on algorithm (§9.8.9.1 exponent rules already cover dimension obligations) | — |

### Summary counts

| Classification | Count |
|----------------|-------|
| CAPTURED (all UNGATED) | 4 |
| STRUCTURAL (type shape / library assert, no runtime gate needed) | 6 |
| SPEC-SILENT (tool territory — explicit) | 5 |
| MISSED (genuine language behavioral obligation not in matrix) | 0 |

### Honesty note

This area is predominantly SPEC-SILENT by design. The SysML v2 spec normatively defines
quantity types and dimension rules (§9.8.9.1) and library shapes (§9.4, §9.8); it explicitly
delegates numerical solving (`Integrate` — *"actual implementation should be given by a
solver"*), zero-crossing detection (*"may notify"*), unit conversion chaining, and
interpolation algorithms entirely to tools. The boundary is sharp: the language layer owns
dimension-matching and monotonicity invariants; the tool layer owns everything numerical.
No missed language behavioral obligations were found. The four UNGATED obligations
(`mref-dimension`, `quantity-arithmetic`, `sampled-function-monotonic`, `interpolate-null`)
remain the correct candidate gates.
