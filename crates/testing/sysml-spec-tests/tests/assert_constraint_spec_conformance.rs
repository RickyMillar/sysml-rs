//! ACSC — Asserted / negated-constraint spec-conformance harness.
//!
//! Companion to `constraint_spec_conformance.rs`. That pilot file deferred the
//! two *asserted-constraint* obligations (`assert-constraint-must-be-true` and
//! `negated-assert-must-be-false`) because the asserted-vs-plain VERDICT mapping
//! lives in the per-instance check / verdict layer rather than the raw
//! eval-boolean layer. This file gates those two rows specifically and
//! CONFIRMS GAP-1: the negation of a plain `assert not constraint {C}` is
//! dropped at the constraint-evaluation layer — `C` is evaluated un-inverted.
//!
//! Convention (matches `constraint_spec_conformance.rs`): every test encodes
//! ONE spec obligation and asserts the engine's CURRENT behavior, carrying two
//! markers on their own lines:
//!
//! - `// OBL:` — the obligation id from
//!   `crates/testing/sysml-spec-tests/spec-obligations/constraints-expressions.md`.
//! - `// VERDICT: CONFORMS` — current behavior satisfies the obligation (test
//!   runs and passes).
//! - `// VERDICT: DIVERGES — <reason>` — a CONFIRMED conformance gap. The test
//!   asserts the SPEC-CORRECT expectation and is `#[ignore]`d pending the fix,
//!   so a plain `cargo test` shows it as ignored/pending; closing the gap =
//!   delete the `#[ignore]` and the test goes green. `cargo test -- --ignored`
//!   runs it and it FAILS against current (wrong) behavior — that failure is the
//!   proof the gap is real.
//!
//! Obligations gated here (tracker §"Asserted Constraints", SysML §7.20):
//! - `assert-constraint-must-be-true` (GAP-2, DEFERRED in the tracker): a
//!   non-negated assert constraint asserts its result is true at all times.
//! - `negated-assert-must-be-false` (GAP-1, DIVERGES): a negated
//!   `assert not constraint {C}` asserts `C` is FALSE; a true `C` is the
//!   inconsistency. Spec: *"An assert constraint usage can also be negated,
//!   which means that the given constraint is asserted to be false rather than
//!   true."* (GOSPEL).
//!
//! Method: parse `assert constraint` / `assert not constraint` from real
//! `.sysml` source with `TreeSitterParser`, run `sysml_core::elaborate::elaborate`
//! so `isNegated` is normalized to a bool (exactly the pipeline
//! `sysml-runtime`'s `e2e_negated_assert_constraint` exercises), then evaluate.
//! Two evaluation surfaces are checked so the gap is pinned at BOTH the layers
//! the tracker names:
//!   1. eval-layer: `extract_and_precompile` → `evaluate_all` (the constraint
//!      monitor / check command bottom out here).
//!   2. real check path: `ModelCompiler::evaluate_constraints_per_instance`
//!      (per-occurrence verdicts; the constraint inside a `part` so the
//!      occurrence path runs and the concrete `x` is seeded).
//! NO production code changes — this file measures.

use sysml_core::Value;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;
use sysml_runtime::constraints::{extract_and_precompile, EvalContext, EvaluationResult};

// ---------------------------------------------------------------------------
// Harness helpers
// ---------------------------------------------------------------------------

/// Parse a sysml source string into a ModelGraph, asserting it parses cleanly.
fn parse_source(source: &str) -> sysml_core::ModelGraph {
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new(
        "assert_constraint_spec_conformance.sysml",
        source,
    )]);
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.severity == sysml_span::Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "fixture source must parse cleanly, got: {errors:?}"
    );
    result.graph
}

/// Parse + elaborate (so `isNegated` is normalized to bool), then evaluate every
/// discovered constraint at the EVAL layer with a hand-seeded context.
///
/// Returns the single discovered constraint's raw eval result. Asserts exactly
/// one constraint was discovered so each fixture isolates one obligation.
fn eval_layer_verdict(source: &str, bindings: &[(&str, Value)]) -> EvaluationResult {
    let mut graph = parse_source(source);
    sysml_core::elaborate::elaborate(&mut graph);
    let precompiled = extract_and_precompile(&graph);
    assert_eq!(
        precompiled.compiled_count(),
        1,
        "fixture must contribute exactly one evaluable constraint"
    );
    let mut ctx = EvalContext::new();
    for (name, value) in bindings {
        ctx.set(*name, value.clone());
    }
    precompiled.evaluate_all(&ctx).remove(0)
}

/// Run the REAL per-instance check path: parse + elaborate, build a
/// `ModelCompiler`, and evaluate the (single) constraint per occurrence. The
/// constraint lives inside a `part` so a concrete `x` is seeded from the owner
/// scope automatically. Returns the single occurrence's eval result.
fn per_instance_verdict(source: &str) -> EvaluationResult {
    let mut graph = parse_source(source);
    sysml_core::elaborate::elaborate(&mut graph);
    let precompiled = extract_and_precompile(&graph);
    assert_eq!(
        precompiled.compiled_count(),
        1,
        "fixture must contribute exactly one evaluable constraint"
    );
    let compiler = ModelCompiler::new(graph);
    let base = EvalContext::new();
    let mut results = compiler
        .evaluate_constraints_per_instance(&precompiled, &base)
        .expect("constraint evaluation");
    assert_eq!(
        results.len(),
        1,
        "fixture must yield exactly one per-instance verdict, got {}",
        results.len()
    );
    results.remove(0).result
}

// ===========================================================================
// OBL-3 — assert-constraint-must-be-true  (tracker GAP-2, DEFERRED)
// A non-negated `assert constraint {C}` asserts C is true at all times.
// The eval layer reports the raw boolean of C correctly; whether an asserted-
// and-false constraint maps to a Fail *verdict* is the deferred check-layer
// concern. These two tests pin the raw-boolean baseline the verdict mapping
// will build on.
// Spec: SysML §7.20 "Asserted Constraints".
// ===========================================================================

#[test]
fn obl3_assert_constraint_true_is_satisfied() {
    // OBL: assert-constraint-must-be-true
    // VERDICT: CONFORMS
    // assert constraint {x < 10}, x = 5 → inner TRUE → satisfied (baseline).
    let r = eval_layer_verdict(
        "part def P {\n    attribute x;\n    assert constraint c { x < 10 }\n}\n",
        &[("x", Value::Int(5))],
    );
    assert!(r.satisfied, "asserted true constraint should be satisfied");
    assert!(!r.inconclusive);
}

#[test]
fn obl3_assert_constraint_false_is_not_satisfied() {
    // OBL: assert-constraint-must-be-true
    // VERDICT: CONFORMS
    // assert constraint {x < 10}, x = 50 → inner FALSE → not-satisfied. The
    // asserted-true-but-violated → Fail-verdict mapping is the deferred
    // check-layer concern (GAP-2); the raw boolean here is correct.
    let r = eval_layer_verdict(
        "part def P {\n    attribute x;\n    assert constraint c { x < 10 }\n}\n",
        &[("x", Value::Int(50))],
    );
    assert!(!r.satisfied, "asserted-but-false constraint is not satisfied");
    assert!(!r.inconclusive, "a decided false is not inconclusive");
}

// ===========================================================================
// OBL-4 — negated-assert-must-be-false  (tracker GAP-1, DIVERGES)  [KEY GATE]
// A negated `assert not constraint {C}` asserts C is FALSE. Per spec the
// verdict should INVERT C: inner-true ⇒ violated, inner-false ⇒ satisfied.
// ACTUAL: negation is dropped at the constraint-evaluation layer (ConstraintIR
// carries no isNegated; only RequirementConstraintIR does), so C is evaluated
// un-inverted. These tests assert the ACTUAL (un-inverted) behavior and mark it
// DIVERGES — this is the CONFIRMATION of GAP-1.
// Spec: SysML §7.20: "An assert constraint usage can also be negated, which
// means that the given constraint is asserted to be false rather than true."
// ===========================================================================

#[test]
fn obl4_negated_assert_inner_true_should_be_violated() {
    // OBL: negated-assert-must-be-false
    // VERDICT: CONFORMS — GAP-1 closed: ConstraintIR.is_negated inverts the decided verdict (SysML §7.20).
    // assert not constraint {x < 10}, x = 5 → inner (x<10) TRUE.
    // SPEC: the negated assertion is VIOLATED (C asserted false but C is true).
    let r = eval_layer_verdict(
        "part def P {\n    attribute x;\n    assert not constraint c { x < 10 }\n}\n",
        &[("x", Value::Int(5))],
    );
    assert!(
        !r.satisfied,
        "SPEC §7.20: negated assertion with inner-true (x<10, x=5) must be VIOLATED"
    );
    assert!(!r.inconclusive);
}

#[test]
fn obl4_negated_assert_inner_false_should_be_satisfied() {
    // OBL: negated-assert-must-be-false
    // VERDICT: CONFORMS — GAP-1 closed: ConstraintIR.is_negated inverts the decided verdict (SysML §7.20).
    // assert not constraint {x < 10}, x = 50 → inner (x<10) FALSE.
    // SPEC: the negated assertion is SATISFIED (C asserted false and C is false).
    let r = eval_layer_verdict(
        "part def P {\n    attribute x;\n    assert not constraint c { x < 10 }\n}\n",
        &[("x", Value::Int(50))],
    );
    assert!(
        r.satisfied,
        "SPEC §7.20: negated assertion with inner-false (x<10, x=50) must be SATISFIED"
    );
    assert!(!r.inconclusive);
}

#[test]
fn obl4_negated_assert_unbound_is_inconclusive() {
    // OBL: negated-assert-must-be-false
    // VERDICT: CONFORMS
    // assert not constraint {x < 10}, x unbound → inner is undecidable, so the
    // verdict is Inconclusive regardless of whether negation is applied. The
    // gap (negation) does not affect this case: both spec and actual yield
    // Inconclusive (an unbound operand can't be inverted into a real verdict).
    let r = eval_layer_verdict(
        "part def P {\n    attribute x;\n    assert not constraint c { x < 10 }\n}\n",
        &[], // x never bound
    );
    assert!(
        r.inconclusive,
        "unbound feature ⇒ inconclusive (negation cannot manufacture a verdict)"
    );
    assert!(!r.satisfied);
}

// ===========================================================================
// OBL-4 — same gap, pinned at the REAL per-instance check path.
// `ModelCompiler::evaluate_constraints_per_instance` is the per-occurrence
// verdict surface the `sysml.constraint.check` command uses. The constraint
// lives inside a `part` with a concrete `x`, so the occurrence path seeds `x`
// from the owner scope. If negation were applied at the check/verdict layer it
// would show here; it is not.
// ===========================================================================

#[test]
fn obl4_negated_assert_inner_true_should_be_violated_in_per_instance_check() {
    // OBL: negated-assert-must-be-false
    // VERDICT: CONFORMS — GAP-1 closed: per-instance check path inherits the inverted verdict.
    // part p { x = 5; assert not constraint { x < 10 } } → inner (x<10) TRUE.
    // SPEC: VIOLATED. (engine: satisfied, un-inverted, seeded x=5.)
    let r = per_instance_verdict(
        "part def P {\n\
         \x20   attribute x = 5;\n\
         \x20   assert not constraint c { x < 10 }\n\
         }\n\
         part p : P;\n",
    );
    assert!(
        !r.satisfied,
        "SPEC §7.20: per-instance negated assertion, inner-true (x=5) must be VIOLATED"
    );
    assert!(!r.inconclusive);
}

#[test]
fn obl4_negated_assert_inner_false_should_be_satisfied_in_per_instance_check() {
    // OBL: negated-assert-must-be-false
    // VERDICT: CONFORMS — GAP-1 closed: per-instance check path inherits the inverted verdict.
    // part p { x = 50; assert not constraint { x < 10 } } → inner (x<10) FALSE.
    // SPEC: SATISFIED. (engine: not-satisfied, un-inverted, seeded x=50.)
    let r = per_instance_verdict(
        "part def P {\n\
         \x20   attribute x = 50;\n\
         \x20   assert not constraint c { x < 10 }\n\
         }\n\
         part p : P;\n",
    );
    assert!(
        r.satisfied,
        "SPEC §7.20: per-instance negated assertion, inner-false (x=50) must be SATISFIED"
    );
    assert!(!r.inconclusive);
}

// ===========================================================================
// Matrix summary — self-scans this file and prints verdict counts, and asserts
// GAP-1 is confirmed (at least one DIVERGES marked against
// negated-assert-must-be-false).
// ===========================================================================

#[test]
fn acsc_matrix_summary() {
    let src = include_str!("assert_constraint_spec_conformance.rs");
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
        "ACSC asserted/negated-constraint matrix: {} gated obligations — \
         {conforms} CONFORMS, {diverges} DIVERGES, {unimpl} UNIMPLEMENTED",
        verdicts.len()
    );
    // GAP-1 (negated-assert-must-be-false) is now CLOSED: the four
    // negated-assert gates flipped DIVERGES → CONFORMS once
    // `ConstraintIR.is_negated` was added and the verdict inverted at the eval
    // chokepoint (SysML §7.20). No DIVERGES rows should remain in this file; if
    // any reappear, a regression reopened the gap.
    assert_eq!(
        diverges, 0,
        "GAP-1 closed — no negated-assert DIVERGES should remain"
    );
    assert!(
        verdicts.len() >= 7,
        "expected ≥7 verdict-marked gates in the asserted-constraint file"
    );
}
