//! QSC — Quantity / physics / sampled-function spec-conformance harness.
//!
//! Sibling to `constraint_spec_conformance.rs` (CSC); same convention. Each
//! test encodes ONE spec-defined *language* obligation in the ODE / physics /
//! quantities area and asserts the engine's CURRENT behavior against it,
//! carrying a verdict marker on its own line:
//!
//! - `// VERDICT: CONFORMS` — current behavior satisfies the obligation.
//! - `// VERDICT: DIVERGES — <reason>` — the assertion pins what the engine
//!   ACTUALLY does today, which differs from the spec obligation. It fails only
//!   if behavior silently changes; flip + re-verdict when a fix closes the gap.
//! - `// VERDICT: UNIMPLEMENTED — <missing>` — the obligation has no engine
//!   surface; the test pins the absence.
//!
//! Each test names its obligation with an `// OBL:` line whose id matches the
//! obligation tracker at
//! `crates/testing/sysml-spec-tests/spec-obligations/ode-physics.md` (the
//! authority for spec citations).
//!
//! KEY SCOPE NOTE (from the tracker's key finding): numerical ODE *solving*
//! (integration algorithm, step control, zero-crossing detection, stiffness) is
//! SPEC-SILENT — explicitly delegated to tools — and is therefore NOT gated here
//! as conformance. Only the genuine *language* obligations are gated:
//!   1. dimensional consistency of quantity arithmetic / comparison (§9.8.9.1),
//!   2. sampled-function domain monotonicity (§9.4.3.2.6),
//!   3. interpolate out-of-bounds behavior (§9.4.3.2.2).
//!
//! Harness rules (match CSC): pure runtime only. Quantity arithmetic is gated at
//! the stable `ExpressionEvaluator` over hand-built `ExprIR` with `Value::Quantity`
//! operands (the path the constraint monitor / check command bottom out in).
//! SampledFunction / Interpolate are gated through the same evaluator via the
//! `ExprIR::FunctionCall` stdlib dispatch (`SampledFunction(...)`, `Interpolate(...)`).
//! NO LSP, NO SysmlService, NO production code changes — this file measures.
//!
//! The summary test (`qsc_matrix_summary`) self-scans this file via
//! `include_str!` and prints the CONFORMS / DIVERGES / UNIMPLEMENTED counts.

use sysml_core::physics::DimensionVector;
use sysml_core::Value;
use sysml_runtime::expressions::{BinOp, EvalContext, EvaluationError, ExprIR, ExpressionEvaluator};

// ---------------------------------------------------------------------------
// Harness helpers
// ---------------------------------------------------------------------------

/// Evaluate a single `ExprIR` against an empty context, returning the raw result.
fn eval(expr: &ExprIR) -> Result<Value, EvaluationError> {
    ExpressionEvaluator::new().eval(expr, &EvalContext::new())
}

/// A `Value::Quantity` literal with the given magnitude, dimension, and unit.
fn quantity(value: f64, dimension: DimensionVector, unit: &str) -> ExprIR {
    ExprIR::LiteralQuantity {
        value,
        dimension,
        unit: unit.to_string(),
    }
}

/// `left <op> right` over two quantity operands.
fn binop(op: BinOp, left: ExprIR, right: ExprIR) -> ExprIR {
    ExprIR::BinaryOp {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

// ISQ base dimensions used below.
const MASS: DimensionVector = DimensionVector::new(0, 1, 0, 0, 0, 0, 0); // kg
const LENGTH: DimensionVector = DimensionVector::new(1, 0, 0, 0, 0, 0, 0); // m

/// Build a two-arg `SampledFunction(domain_list, range_list)` call expression.
fn sampled_function(domain: &[f64], range: &[f64]) -> ExprIR {
    let to_seq = |xs: &[f64]| ExprIR::Sequence(xs.iter().map(|x| ExprIR::LiteralReal(*x)).collect());
    ExprIR::FunctionCall {
        name: "SampledFunction".to_string(),
        args: vec![to_seq(domain), to_seq(range)],
    }
}

// ===========================================================================
// OBL-Q1 — quantity-arithmetic-dimension-rules (add/subtract)
// §9.8.9.1: +/- require equal quantity dimension.
// FINDING: the engine ENFORCES this — a dimension mismatch is a hard
// `Runtime("dimension mismatch ...")` error, not a silent bare-magnitude op.
// ===========================================================================

#[test]
fn q1_same_dimension_addition_is_allowed() {
    // OBL: quantity-arithmetic-dimension-rules
    // VERDICT: CONFORMS
    let r = eval(&binop(
        BinOp::Add,
        quantity(2.0, MASS, "kg"),
        quantity(3.0, MASS, "kg"),
    ))
    .expect("same-dimension add must succeed");
    match r {
        Value::Quantity { value, dimension, .. } => {
            assert_eq!(value, 5.0);
            assert_eq!(dimension, MASS, "result keeps the shared dimension");
        }
        other => panic!("expected Quantity, got {other:?}"),
    }
}

#[test]
fn q1_different_dimension_addition_is_rejected() {
    // OBL: quantity-arithmetic-dimension-rules
    // VERDICT: CONFORMS
    // mass + length: the engine refuses with a dimension-mismatch error rather
    // than adding the bare magnitudes — this IS the §9.8.9.1 obligation.
    let r = eval(&binop(
        BinOp::Add,
        quantity(2.0, MASS, "kg"),
        quantity(3.0, LENGTH, "m"),
    ));
    match r {
        Err(EvaluationError::Runtime(msg)) => {
            assert!(
                msg.contains("dimension mismatch"),
                "expected dimension-mismatch diagnostic, got: {msg}"
            );
        }
        other => panic!("expected a dimension-mismatch Runtime error, got {other:?}"),
    }
}

// ===========================================================================
// OBL-Q2 — quantity-arithmetic-dimension-rules (multiply)
// §9.8.9.1: × multiplies dimensions (exponents sum).
// ===========================================================================

#[test]
fn q2_multiplication_sums_dimensions() {
    // OBL: quantity-arithmetic-dimension-rules
    // VERDICT: CONFORMS
    // length * length → area (length exponent 2). Distinct dimensions are
    // permitted under × (unlike +), and the exponents add.
    let r = eval(&binop(
        BinOp::Multiply,
        quantity(2.0, LENGTH, "m"),
        quantity(3.0, LENGTH, "m"),
    ))
    .expect("multiply must succeed across dimensions");
    match r {
        Value::Quantity { value, dimension, .. } => {
            assert_eq!(value, 6.0);
            assert_eq!(dimension, DimensionVector::new(2, 0, 0, 0, 0, 0, 0));
        }
        other => panic!("expected Quantity, got {other:?}"),
    }
}

// ===========================================================================
// OBL-Q3 — mref-dimension-must-match-attribute (comparison surface)
// §9.8.9.1: relational ops require the same quantity dimension. This is the
// closest runtime-observable proxy for "a supplied mRef must match the
// attribute's quantity dimension" — a same-dimension/different-dimension
// comparison.
// FINDING: the engine ENFORCES dimensional consistency on comparison too
// (hard error on mismatch); same-dimension comparison decides normally.
// ===========================================================================

#[test]
fn q3_same_dimension_comparison_decides() {
    // OBL: mref-dimension-must-match-attribute
    // VERDICT: CONFORMS
    let r = eval(&binop(
        BinOp::LessThan,
        quantity(2.0, MASS, "kg"),
        quantity(5.0, MASS, "kg"),
    ))
    .expect("same-dimension comparison must succeed");
    assert_eq!(r, Value::Bool(true));
}

#[test]
fn q3_different_dimension_comparison_is_rejected() {
    // OBL: mref-dimension-must-match-attribute
    // VERDICT: CONFORMS
    // Comparing a mass to a length is refused — the engine does NOT silently
    // compare bare magnitudes across incompatible dimensions.
    let r = eval(&binop(
        BinOp::LessThan,
        quantity(2.0, MASS, "kg"),
        quantity(5.0, LENGTH, "m"),
    ));
    match r {
        Err(EvaluationError::Runtime(msg)) => {
            assert!(
                msg.contains("dimension mismatch"),
                "expected dimension-mismatch diagnostic, got: {msg}"
            );
        }
        other => panic!("expected a dimension-mismatch Runtime error, got {other:?}"),
    }
}

// ===========================================================================
// OBL-Q4 — sampled-function-must-be-monotonic
// §9.4.3.2.6 / SampledFunctions.sysml:40-44: a SampledFunction's domain values
// must be strictly increasing or decreasing.
// FINDING: a runtime surface EXISTS (stdlib `SampledFunction(...)`), but it
// enforces monotonicity only partially: it SORTS the domain ascending and then
// rejects DUPLICATE domain values. So a duplicate (non-strict) domain is
// rejected (conforms to the strictness intent), but a strictly-*decreasing*
// input is silently re-sorted ascending rather than honored/validated as-given.
// ===========================================================================

#[test]
fn q4_duplicate_domain_value_is_rejected() {
    // OBL: sampled-function-must-be-monotonic
    // VERDICT: CONFORMS
    // Two samples at the same domain point violate strict monotonicity and are
    // refused.
    let r = eval(&sampled_function(&[0.0, 1.0, 1.0], &[10.0, 20.0, 30.0]));
    match r {
        Err(EvaluationError::TypeError(msg)) => {
            assert!(
                msg.contains("monotonic") || msg.contains("duplicate"),
                "expected a monotonicity/duplicate diagnostic, got: {msg}"
            );
        }
        other => panic!("expected a monotonicity TypeError, got {other:?}"),
    }
}

#[test]
fn q4_strictly_decreasing_domain_is_resorted_not_validated() {
    // OBL: sampled-function-must-be-monotonic
    // VERDICT: CONFORMS — GAP-PHYS (Q4) closed: a strictly-decreasing domain is
    // accepted and its order preserved (no re-sort); only non-monotonic domains
    // are rejected. SampledFunctions.sysml:30-43 / §9.4.3.2.6.
    let r = eval(&sampled_function(&[2.0, 1.0, 0.0], &[20.0, 10.0, 0.0]))
        .expect("decreasing domain is monotonic and must be accepted");
    match r {
        Value::Map(map) => {
            let domain = map.get("domain").expect("SampledFunction has a domain list");
            assert_eq!(
                domain,
                &Value::List(vec![
                    Value::Float(2.0),
                    Value::Float(1.0),
                    Value::Float(0.0),
                ]),
                "spec admits a strictly-decreasing domain as monotonic; the \
                 caller's order must be preserved, not re-sorted ascending"
            );
        }
        other => panic!("expected a SampledFunction Map, got {other:?}"),
    }
}

// ===========================================================================
// OBL-Q5 — interpolate-returns-null-out-of-bounds
// §9.4.3.2.2 / SampledFunctions.sysml:80-90: `Interpolate` returns null (no
// extrapolation) for an out-of-bounds input.
// FINDING: a runtime surface EXISTS (stdlib `Interpolate`/`interpolateLinear`),
// but it CLAMPS to the nearest edge range value out of bounds rather than
// returning null — a deliberate tool choice (ODE edge-value continuity).
// ===========================================================================

#[test]
fn q5_interpolate_in_bounds_is_linear() {
    // OBL: interpolate-returns-null-out-of-bounds
    // VERDICT: CONFORMS
    // In-bounds interpolation is plain linear (this part matches the spec calc).
    let sf = sampled_function(&[0.0, 1.0], &[0.0, 10.0]);
    let r = eval(&ExprIR::FunctionCall {
        name: "Interpolate".to_string(),
        args: vec![sf, ExprIR::LiteralReal(0.5)],
    })
    .expect("in-bounds interpolation must succeed");
    assert_eq!(r, Value::Float(5.0));
}

#[test]
fn q5_interpolate_out_of_bounds_clamps_instead_of_null() {
    // OBL: interpolate-returns-null-out-of-bounds
    // VERDICT: CONFORMS — GAP-PHYS (Q5) closed: `Interpolate` now returns null
    // out of bounds (no extrapolation), SampledFunctions.sysml:80-84 / §9.4.3.2.2.
    // (The internal `interpolateLinear` ODE helper still clamps for integration
    // edge-continuity — a flagged tool divergence, not gated here.)
    let sf = sampled_function(&[0.0, 1.0], &[0.0, 10.0]);
    let r = eval(&ExprIR::FunctionCall {
        name: "Interpolate".to_string(),
        args: vec![sf, ExprIR::LiteralReal(5.0)], // x = 5 is well above the domain max (1.0)
    })
    .expect("out-of-bounds interpolation does not error today");
    assert_eq!(
        r,
        Value::Null,
        "spec requires null out of bounds (no extrapolation); engine must not \
         clamp to the last range value"
    );
}

// ===========================================================================
// SCOPE MARKER — numerical ODE solving is SPEC-SILENT (tool territory).
// Recorded here as an explicit non-gate so the matrix documents the boundary:
// integration algorithm, step-size control, and zero-crossing detection are
// delegated to solvers by the spec and are NOT conformance obligations.
// ===========================================================================

#[ignore = "UNIMPL: ODE integration / step control / zero-crossing detection are \
            SPEC-SILENT (explicitly delegated to tools) — no language obligation to gate"]
#[test]
fn ode_numerical_solving_is_spec_silent_not_gated() {
    // OBL: ode-solver-delegation
    // VERDICT: UNIMPLEMENTED — by spec design: ODE integration / step control /
    // zero-crossing detection are SPEC-SILENT (delegated to tools), so there is
    // no *language* obligation to gate. This marker documents the boundary; the
    // numerical engine's correctness is a numerical-validation concern.
    assert!(true);
}

// ===========================================================================
// Matrix summary — self-scans this file and prints verdict counts.
// ===========================================================================

#[test]
fn qsc_matrix_summary() {
    let src = include_str!("quantity_spec_conformance.rs");
    let verdicts: Vec<&str> = src
        .lines()
        .map(|l| l.trim())
        .filter(|l| l.starts_with("// VERDICT: "))
        .collect();
    let conforms = verdicts
        .iter()
        .filter(|l| l.starts_with("// VERDICT: CONFORMS"))
        .count();
    let diverges = verdicts
        .iter()
        .filter(|l| l.starts_with("// VERDICT: DIVERGES"))
        .count();
    let unimpl = verdicts
        .iter()
        .filter(|l| l.starts_with("// VERDICT: UNIMPLEMENTED"))
        .count();
    println!(
        "QSC quantity/physics matrix: {} gated obligations — \
         {conforms} CONFORMS, {diverges} DIVERGES, {unimpl} UNIMPLEMENTED",
        verdicts.len()
    );
    assert!(
        verdicts.len() >= 8,
        "expected >=8 verdict-marked gates in the quantity/physics file"
    );
}
