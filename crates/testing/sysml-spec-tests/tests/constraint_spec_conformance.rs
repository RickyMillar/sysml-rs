//! CSC — Constraint & Expression spec-conformance harness.
//!
//! Pilot area of the spec-derived semantic-conformance sweep (Deliverable 2).
//! Sibling to `runtime_spec_conformance.rs` (which covers transfers / triggers /
//! clocks / ports / flows); this file covers the **constraints & expressions**
//! semantic area. Same convention: every test encodes ONE spec-defined
//! obligation and asserts the engine's CURRENT behavior against it, carrying a
//! verdict marker on its own line:
//!
//! - `// VERDICT: CONFORMS` — current behavior satisfies the obligation.
//! - `// VERDICT: DIVERGES — <reason>` — the test asserts what the engine
//!   ACTUALLY does today, which differs from the spec obligation. It fails only
//!   if behavior silently changes. When a later fix-wave closes the gap, the
//!   assertion is flipped and the verdict updated.
//! - `// VERDICT: UNIMPLEMENTED — <missing>` — the obligation has no engine
//!   surface; the test pins the absence.
//!
//! Each test names the obligation it gates with an `// OBL:` line whose id
//! matches the obligation tracker at
//! `crates/testing/sysml-spec-tests/spec-obligations/constraints-expressions.md`.
//! That tracker is the authority for spec citations; this file is the gate.
//!
//! Spec sources (verified against the spec HTML, cited per obligation in the
//! tracker):
//! - SysML §7.20 "Constraints" (`SysML-spec-r2025-04_REF.html`)
//! - KerML §7.4.8 "Predicates", §8.3.4.8.5 "FeatureReferenceExpression::evaluate"
//! - `sysml.library/Systems Library/Constraints.sysml`
//!   (`ConstraintCheck :> BooleanEvaluation`)
//! - `sysml.library/Kernel Libraries/Kernel Semantic Library/Performances.kerml`
//!   (`BooleanEvaluation` returns `Boolean[1]`)
//!
//! Harness rules (match `runtime_spec_conformance.rs`): pure runtime only.
//! Evaluation-semantics obligations are gated at the stable
//! `sysml_runtime::constraints::evaluate(ConstraintIR, EvalContext)` layer with
//! hand-seeded contexts — decoupled from the in-flight per-instance binding /
//! verdict machinery (`compiler::evaluate_constraints_per_instance`, the check
//! command). Structural obligations are gated by parsing real `.sysml` source
//! and asserting discovery via `extract_constraints`. NO LSP, NO SysmlService,
//! NO production code changes — this file measures.
//!
//! The summary test (`csc_matrix_summary`) self-scans this file via
//! `include_str!` and prints the CONFORMS / DIVERGES / UNIMPLEMENTED counts.

use sysml_core::Value;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::constraints::{evaluate, extract_constraints, EvalContext};
use sysml_runtime::ConstraintIR;

// ---------------------------------------------------------------------------
// Harness helpers
// ---------------------------------------------------------------------------

/// Evaluate a constraint expression string against a seeded context.
///
/// `bindings` are the feature values visible to the expression. This drives the
/// same stable code path the constraint monitor and check command bottom out in
/// (`ConstraintIR` → `ExpressionEvaluator` → `EvaluationResult`).
fn eval_expr(expr: &str, bindings: &[(&str, Value)]) -> sysml_runtime::constraints::EvaluationResult
{
    let mut ctx = EvalContext::new();
    for (name, value) in bindings {
        ctx.set(*name, value.clone());
    }
    evaluate(&ConstraintIR::new(expr), &ctx)
}

/// Parse a sysml source string into an (un-elaborated) ModelGraph.
///
/// Parse errors fail the test immediately — every fixture here must be syntax
/// the tree-sitter grammar accepts, otherwise the case measures the parser
/// instead of the runtime.
fn parse_source(source: &str) -> sysml_core::ModelGraph {
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new("constraint_spec_conformance.sysml", source)]);
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

// ===========================================================================
// OBL-1 — constraint-result-boolean
// A constraint produces exactly one Boolean result; a non-Boolean expression
// is not a valid constraint verdict.
// Spec: KerML §7.4.8.1 "Predicates are functions whose result is a single
// Boolean value"; Performances.kerml BooleanEvaluation returns Boolean[1].
// ===========================================================================

#[test]
fn obl1_boolean_expression_yields_concrete_verdict() {
    // OBL: constraint-result-boolean
    // VERDICT: CONFORMS
    let r = eval_expr("x < 10", &[("x", Value::Int(3))]);
    assert!(!r.inconclusive, "a fully-bound Boolean constraint is decidable");
    assert!(r.satisfied);
}

#[test]
fn obl1_non_boolean_expression_is_not_a_verdict() {
    // OBL: constraint-result-boolean
    // VERDICT: CONFORMS
    // A numeric (non-Boolean) result is refused as a verdict: the engine marks
    // it inconclusive with an "expected boolean" diagnostic rather than coercing
    // it. This upholds the Boolean[1] result obligation.
    let r = eval_expr("3 + 4", &[]);
    assert!(
        r.inconclusive,
        "non-Boolean constraint result must not be treated as satisfied/violated"
    );
    assert!(!r.satisfied);
    assert!(
        r.diagnostics.iter().any(|d| d.message.contains("expected boolean")),
        "non-Boolean result should diagnose the Boolean[1] obligation"
    );
}

// ===========================================================================
// OBL-2 — constraint-satisfied-iff-true
// "a constraint usage is satisfied if its expression evaluates to true and is
// violated otherwise." — SysML §7.20.
// ===========================================================================

#[test]
fn obl2_satisfied_when_expression_true() {
    // OBL: constraint-satisfied-iff-true
    // VERDICT: CONFORMS
    let r = eval_expr("mass < 10", &[("mass", Value::Int(5))]);
    assert!(r.satisfied);
    assert!(!r.inconclusive);
}

#[test]
fn obl2_violated_when_expression_false() {
    // OBL: constraint-satisfied-iff-true
    // VERDICT: CONFORMS
    let r = eval_expr("mass < 10", &[("mass", Value::Int(50))]);
    assert!(!r.satisfied, "false expression ⇒ violated");
    assert!(
        !r.inconclusive,
        "a violation is a decided verdict, not inconclusive"
    );
}

// ===========================================================================
// OBL-8 — feature-ref-resolves-to-bound-value
// A FeatureReferenceExpression evaluates to the value bound to the referenced
// feature. — KerML §8.3.4.8.5 (FeatureReferenceExpression::evaluate).
// Gated at the eval layer: the identifier resolves to its context binding.
// ===========================================================================

#[test]
fn obl8_feature_reference_resolves_to_its_bound_value() {
    // OBL: feature-ref-resolves-to-bound-value
    // VERDICT: CONFORMS
    // Same expression, two different bound values ⇒ two different verdicts:
    // the reference is genuinely resolved against the context, not constant-folded.
    let lo = eval_expr("speed < 100", &[("speed", Value::Float(80.0))]);
    let hi = eval_expr("speed < 100", &[("speed", Value::Float(120.0))]);
    assert!(lo.satisfied);
    assert!(!hi.satisfied);
}

// ===========================================================================
// OBL-9 — unbound-feature-yields-inconclusive-not-false  [KEY GAP-FLAG]
// KerML §8.3.4.8.5: an unresolved FeatureReferenceExpression evaluates to the
// empty list. The spec is SILENT on what an ordering comparison ("<", ">", ...)
// does with an empty operand. Our tool choice: report Inconclusive (honest
// "could not determine"), NOT a false/violated verdict. This is spec-silent but
// spec-consistent — flagged for the director as a CONFORMS-by-design item with
// no normative obligation to gate against.
// ===========================================================================

#[test]
fn obl9_unbound_feature_is_inconclusive_not_violated() {
    // OBL: unbound-feature-yields-inconclusive-not-false
    // VERDICT: CONFORMS
    let r = eval_expr("mass < 10", &[]); // mass never bound
    assert!(
        r.inconclusive,
        "an unbound feature reference ⇒ inconclusive (cannot determine)"
    );
    assert!(
        !r.satisfied,
        "inconclusive is reported via satisfied=false, but is NOT a violation — \
         consumers must read `inconclusive` first"
    );
}

// ===========================================================================
// EXPR-1 — core-operator-semantics
// Constraint expressions are KerML Boolean expressions; the operator set
// (comparison, logical, arithmetic) must evaluate per KerML expression
// semantics. — KerML §7.4.8 / KerMLExpressions.xtext.
// ===========================================================================

#[test]
fn expr1_comparison_logical_and_arithmetic_operators() {
    // OBL: core-operator-semantics
    // VERDICT: CONFORMS
    assert!(eval_expr("1 + 2 == 3", &[]).satisfied);
    assert!(eval_expr("10 > 3", &[]).satisfied);
    assert!(eval_expr("3 >= 3", &[]).satisfied);
    assert!(!eval_expr("3 > 3", &[]).satisfied);
    // Conjunction / disjunction over a bound feature.
    let band = eval_expr("x >= 10 and x <= 20", &[("x", Value::Int(15))]);
    assert!(band.satisfied);
    let out = eval_expr("x >= 10 and x <= 20", &[("x", Value::Int(25))]);
    assert!(!out.satisfied && !out.inconclusive);
}

// ===========================================================================
// OBL-5 / OBL-6 — structural discovery (constraint vs assert constraint)
// A ConstraintUsage and an AssertConstraintUsage are both discovered as
// constraints by the runtime. This pins the parse→ConstraintIR wiring so the
// constraint surface cannot silently vanish. (The asserted-vs-plain *meaning*
// obligation, OBL-3/OBL-4, is tracked separately — see the tracker.)
// ===========================================================================

#[test]
fn obl56_plain_and_assert_constraints_are_both_discovered() {
    // OBL: constraint-usage-discovered
    // VERDICT: CONFORMS
    let graph = parse_source(
        "part def Vehicle {\n\
         \x20   attribute speed;\n\
         \x20   constraint plain { speed < 100 }\n\
         \x20   assert constraint asserted { speed > 0 }\n\
         }\n",
    );
    let set = extract_constraints(&graph);
    assert!(
        set.constraints.len() >= 2,
        "both `constraint` and `assert constraint` must be discovered, got {}",
        set.constraints.len()
    );
}

// ===========================================================================
// Matrix summary — self-scans this file and prints verdict counts.
// ===========================================================================

#[test]
fn csc_matrix_summary() {
    let src = include_str!("constraint_spec_conformance.rs");
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
        "CSC constraint/expression matrix: {} gated obligations — \
         {conforms} CONFORMS, {diverges} DIVERGES, {unimpl} UNIMPLEMENTED",
        verdicts.len()
    );
    // At least the pilot obligations must be present and accounted for.
    assert!(
        verdicts.len() >= 8,
        "expected ≥8 verdict-marked gates in the constraint pilot"
    );
}
