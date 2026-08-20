//! RSC-REQ — Requirements spec-conformance harness.
//!
//! Fan-out area of the spec-derived semantic-conformance sweep (see
//! `spec-obligations/README.md`). Covers the **requirements** area. Same
//! convention as `constraint_spec_conformance.rs` /
//! `runtime_spec_conformance.rs`: one obligation per test, a `// VERDICT:`
//! marker line per case, an `// OBL:` line tying it to the obligation tracker
//! (`spec-obligations/requirements.md`), and a self-scanning
//! `rscreq_matrix_summary` test.
//!
//! Spec sources (verified; cited in the tracker):
//! - SysML §7.21 "Requirements" (`SysML-spec-r2025-04_REF.html`)
//! - `sysml.library/Systems Library/Requirements.sysml`
//!   (`result = allTrue(assumptions()) implies allTrue(constraints())`)
//!
//! Harness rules: pure runtime. Requirement *satisfaction logic* (the
//! assumption⇒required implication and negation) is gated at the stable
//! `sysml_runtime::constraints::evaluate_requirement` layer with hand-seeded
//! contexts. SUBJECT BINDING obligations (subject parameter, subrequirement
//! subject inheritance, conformance-to-subject-type) are NOT gated here — that
//! surface is the in-flight verification/requirement subject-binding build;
//! the tracker lists them as DEFERRED to avoid collision.

use sysml_core::Value;
use sysml_runtime::constraints::{evaluate_requirement, EvalContext, RequirementConstraintIR};
use sysml_runtime::ConstraintIR;

fn ctx(bindings: &[(&str, Value)]) -> EvalContext {
    let mut c = EvalContext::new();
    for (n, v) in bindings {
        c.set(*n, v.clone());
    }
    c
}

// ===========================================================================
// OBL: requirement-is-constraint-satisfied-iff-true
// "a requirement is satisfied when it evaluates to true." — SysML §7.21.1
// ===========================================================================

#[test]
fn req_satisfied_iff_required_constraint_true() {
    // OBL: requirement-is-constraint-satisfied-iff-true
    // VERDICT: CONFORMS
    let req = RequirementConstraintIR::new("massReq").with_constraint(ConstraintIR::new("mass < 10"));
    assert!(evaluate_requirement(&req, &ctx(&[("mass", Value::Int(5))])).satisfied);
    assert!(!evaluate_requirement(&req, &ctx(&[("mass", Value::Int(50))])).satisfied);
}

// ===========================================================================
// OBL: requirement-result-is-assumption-implies-required
// "the effective constraint for the requirement is a logical implication: if
// all the assumption constraints are true, all the required constraints must be
// true." — SysML §7.21.2; Requirements.sysml result formula.
// ===========================================================================

#[test]
fn req_required_checked_only_when_assumptions_hold() {
    // OBL: requirement-result-is-assumption-implies-required
    // VERDICT: CONFORMS
    let req = RequirementConstraintIR::new("derate")
        .with_assumption(ConstraintIR::new("temp > 40"))
        .with_constraint(ConstraintIR::new("power < 100"));
    // assumption holds, required holds ⇒ satisfied
    assert!(evaluate_requirement(&req, &ctx(&[("temp", Value::Int(50)), ("power", Value::Int(80))])).satisfied);
    // assumption holds, required violated ⇒ NOT satisfied
    assert!(!evaluate_requirement(&req, &ctx(&[("temp", Value::Int(50)), ("power", Value::Int(120))])).satisfied);
}

// ===========================================================================
// OBL: assumption-false-required-vacuously-satisfied
// `false implies X` is true: a failed assumption ⇒ requirement vacuously
// satisfied. — Requirements.sysml result formula (LIBRARY, necessary consequence).
// ===========================================================================

#[test]
fn req_vacuously_satisfied_when_assumption_false() {
    // OBL: assumption-false-required-vacuously-satisfied
    // VERDICT: CONFORMS
    let req = RequirementConstraintIR::new("derate")
        .with_assumption(ConstraintIR::new("temp > 40"))
        .with_constraint(ConstraintIR::new("power < 100"));
    // assumption FALSE (temp=20) ⇒ satisfied even though required would fail (power=120)
    let r = evaluate_requirement(&req, &ctx(&[("temp", Value::Int(20)), ("power", Value::Int(120))]));
    assert!(r.satisfied, "false assumption ⇒ vacuous satisfaction");
}

// ===========================================================================
// OBL: negated-satisfy-requires-not-satisfied
// A negated satisfy requirement usage asserts the requirement evaluates to
// false; the engine inverts the verdict. — SysML §7.21.4;
// notSatisfiedRequirementChecks :> negatedConstraintChecks.
// ===========================================================================

#[test]
fn req_negation_inverts_verdict() {
    // OBL: negated-satisfy-requires-not-satisfied
    // VERDICT: CONFORMS
    let base = RequirementConstraintIR::new("r").with_constraint(ConstraintIR::new("mass < 10"));
    let neg = RequirementConstraintIR::new("r").with_constraint(ConstraintIR::new("mass < 10")).negated();
    let bind = ctx(&[("mass", Value::Int(5))]); // inner constraint TRUE
    assert!(evaluate_requirement(&base, &bind).satisfied);
    assert!(!evaluate_requirement(&neg, &bind).satisfied, "negation inverts a true requirement to unsatisfied");
}

// ===========================================================================
// OBL: requirement-check-result-is-boolean
// Every RequirementCheck result is Boolean. — SysML §8.4.17.1 (RequirementDefinition
// is a kind of ConstraintDefinition; ConstraintDefinition result is Boolean[1]).
// Gated transitively: the verdict is a bool, and an inconclusive required
// constraint surfaces as `inconclusive` rather than a coerced value.
// ===========================================================================

#[test]
fn req_inconclusive_propagates_from_unbound_required_constraint() {
    // OBL: requirement-check-result-is-boolean
    // VERDICT: CONFORMS
    let req = RequirementConstraintIR::new("r").with_constraint(ConstraintIR::new("mass < 10"));
    let r = evaluate_requirement(&req, &ctx(&[])); // mass unbound
    assert!(r.inconclusive, "an unbound required constraint ⇒ inconclusive requirement");
}

// ===========================================================================
// Matrix summary.
// ===========================================================================

#[test]
fn rscreq_matrix_summary() {
    let src = include_str!("requirement_spec_conformance.rs");
    let verdicts: Vec<&str> = src
        .lines()
        .map(|l| l.trim())
        .filter(|l| l.starts_with("// VERDICT: "))
        .collect();
    let conforms = verdicts.iter().filter(|l| l.starts_with("// VERDICT: CONFORMS")).count();
    let diverges = verdicts.iter().filter(|l| l.starts_with("// VERDICT: DIVERGES")).count();
    let unimpl = verdicts.iter().filter(|l| l.starts_with("// VERDICT: UNIMPLEMENTED")).count();
    println!(
        "RSC-REQ requirements matrix: {} gated obligations — {conforms} CONFORMS, {diverges} DIVERGES, {unimpl} UNIMPLEMENTED",
        verdicts.len()
    );
    assert!(verdicts.len() >= 5);
}
