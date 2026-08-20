//! VCSC — Verification & Analysis case spec-conformance harness.
//!
//! Companion to `constraint_spec_conformance.rs`. Same convention: every test
//! encodes ONE spec-defined obligation from
//! `spec-obligations/verification-analysis-cases.md` and asserts the engine's
//! CURRENT behavior against it, carrying a verdict marker on its own line:
//!
//! - `// VERDICT: CONFORMS` — current behavior satisfies the obligation.
//! - `// VERDICT: DIVERGES — <reason>` — the test asserts what the engine
//!   ACTUALLY does today, which differs from the spec obligation; it fails only
//!   if behavior silently changes.
//! - `// VERDICT: UNIMPLEMENTED — <missing>` — the obligation has no engine
//!   surface; the test pins the absence.
//!
//! Each test names the obligation it gates with an `// OBL:` line whose id
//! matches the tracker. The tracker is the authority for spec citations; this
//! file is the gate.
//!
//! Scope discipline: this file gates ONLY obligations NOT already covered by
//! `crates/lang/sysml-runtime/tests/cases_pipeline.rs` (which exhaustively
//! covers verify happy-path, multiple/no requirements, requirement-redefinition
//! binding, referenced-constraint + bare `verify`, subject-attribute binding,
//! feature-reference binding, subject-occurrence binding, and the parsed
//! pipeline). What remains uncovered and is gated here:
//!
//! - `verdict-semantics`  — honest-Inconclusive surfaces at the CASE-level
//!   aggregate verdict (not merely on a single requirement result), and the
//!   worst-wins aggregate does NOT flatten Inconclusive into Fail.
//! - `analysis-case-objective-bound-to-result` — the §7.23.2 asymmetry
//!   (analysis-case objective subject bound to the analysis RESULT) — probed for
//!   an engine surface.
//! - `verdict-criteria-modeled-explicitly` — the spec-silent default when a case
//!   carries no explicit pass/fail criteria.
//!
//! Pure runtime only: programmatic IR construction + the stable
//! `VerificationRunner::verify` / `compile_analysis_case` surfaces. No parser
//! re-test of the binding machinery already gated in cases_pipeline.rs, no LSP,
//! no service, no production code changes — this file MEASURES.

use sysml_runtime::cases::{
    compile_analysis_case, RequirementBinding, RequirementCheck, VerdictKind, VerificationCaseIR,
    VerificationRunner,
};
use sysml_runtime::expressions::{BinOp, EvalContext, ExprIR};
use sysml_core::elaborate::elaborate;
use sysml_core::Value;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

// ---------------------------------------------------------------------------
// Harness helpers
// ---------------------------------------------------------------------------

/// Build a single-requirement verification case directly from IR.
///
/// `constraint` is the requirement's lone constraint expression; the case
/// carries no setup actions and no per-requirement bindings, so whether the
/// constraint's feature references resolve is entirely a function of `ctx`.
fn single_constraint_case(constraint: ExprIR) -> VerificationCaseIR {
    VerificationCaseIR {
        id: "vc".into(),
        name: "Case".into(),
        subject: Some("subj".into()),
        setup_actions: vec![],
        requirements: vec![RequirementCheck {
            id: "req".into(),
            source_element_id: None,
            text: None,
            assumptions: vec![],
            constraints: vec![constraint],
            constraint_element_ids: vec![None],
            compile_errors: vec![],
            subrequirements: vec![],
            bindings: vec![],
            binding_specs: vec![],
        }],
        sub_cases: vec![],
        // No modeled verdict criterion and no case-body attributes — the
        // worst-wins default over requirement checks is exactly what these
        // fixtures pin (G-V1 fields added in 2b2543bb).
        verdict_expression: None,
        bindings: vec![],
    }
}

/// `feature < literal` constraint — the canonical decidable ordering predicate.
fn less_than(feature: &str, rhs: f64) -> ExprIR {
    ExprIR::BinaryOp {
        op: BinOp::LessThan,
        left: Box::new(ExprIR::FeatureRef(feature.into())),
        right: Box::new(ExprIR::LiteralReal(rhs)),
    }
}

fn parse(source: &str) -> sysml_core::ModelGraph {
    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("vcsc.sysml", source)]);
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.severity == sysml_span::Severity::Error)
        .collect();
    assert!(errors.is_empty(), "fixture must parse cleanly, got: {errors:?}");
    elaborate(&mut result.graph);
    result.graph
}

// ===========================================================================
// OBL — verdict-semantics  [honest-Inconclusive AT THE CASE LEVEL]
// §7.24.1: "Inconclusive indicates that a determination could not be made."
// cases_pipeline.rs gates Inconclusive through the parsed compile path; here we
// pin it at the IR/runner boundary and assert it on the CASE-LEVEL aggregate
// (`VerificationResult.verdict`), not just on a single requirement result.
// A SEPARATE path (`evaluate_constraints_with_context`) is known to flatten
// Inconclusive -> Fail; these gates confirm `VerificationRunner::verify` is the
// honest one.
// ===========================================================================

#[test]
fn case_level_verdict_is_inconclusive_when_constraint_feature_unbound() {
    // OBL: verdict-semantics
    // VERDICT: CONFORMS
    // The case's requirement references `speed`, which is never bound in the
    // context. A determination cannot be made -> the CASE verdict is
    // Inconclusive, NOT a silent Fail and NOT a vacuous Pass.
    let case = single_constraint_case(less_than("speed", 100.0));
    let runner = VerificationRunner::new();
    let result = runner.verify(&case, &EvalContext::new());
    assert_eq!(
        result.verdict,
        VerdictKind::Inconclusive,
        "unbound constraint feature -> case verdict Inconclusive (not Fail)"
    );
    assert_ne!(
        result.verdict,
        VerdictKind::Fail,
        "honest-Inconclusive must not be flattened to Fail at the case level"
    );
}

#[test]
fn case_level_inconclusive_survives_aggregate_with_a_passing_requirement() {
    // OBL: verdict-semantics
    // VERDICT: CONFORMS
    // One requirement is decidably satisfied (bound), the other references an
    // unbound feature. Worst-wins aggregation (Error>Fail>Inconclusive>Pass)
    // must surface Inconclusive at the case level — a determination still could
    // not be made for the whole case. This is the precise behavior that a
    // flatten-to-Fail path would corrupt.
    let mut case = single_constraint_case(less_than("massA", 10.0));
    case.requirements.push(RequirementCheck {
        id: "req2".into(),
        source_element_id: None,
        text: None,
        assumptions: vec![],
        constraints: vec![less_than("massB", 10.0)], // massB never bound
        constraint_element_ids: vec![None],
        compile_errors: vec![],
        subrequirements: vec![],
        bindings: vec![],
        binding_specs: vec![],
    });

    let runner = VerificationRunner::new();
    let mut ctx = EvalContext::new();
    ctx.set("massA", Value::Float(3.0)); // req1 passes decisively

    let result = runner.verify(&case, &ctx);
    assert_eq!(
        result.verdict,
        VerdictKind::Inconclusive,
        "Pass aggregated with Inconclusive must stay Inconclusive at the case level"
    );
    // And the per-requirement breakdown is honest about which was which.
    let pass_count = result
        .requirement_results
        .iter()
        .filter(|r| r.verdict == VerdictKind::Pass)
        .count();
    let inconc_count = result
        .requirement_results
        .iter()
        .filter(|r| r.verdict == VerdictKind::Inconclusive)
        .count();
    assert_eq!(pass_count, 1, "the bound requirement passes decisively");
    assert_eq!(inconc_count, 1, "the unbound requirement is inconclusive");
}

#[test]
fn case_level_fail_beats_inconclusive_in_aggregate() {
    // OBL: verdict-semantics
    // VERDICT: CONFORMS
    // A decided violation (Fail) co-occurring with an Inconclusive must surface
    // as Fail (Fail outranks Inconclusive) — the converse of the test above.
    // This pins the worst-wins ordering AT THE CASE LEVEL where cases_pipeline
    // only exercises single-verdict cases.
    let mut case = single_constraint_case(less_than("massA", 10.0)); // will FAIL
    case.requirements.push(RequirementCheck {
        id: "req2".into(),
        source_element_id: None,
        text: None,
        assumptions: vec![],
        constraints: vec![less_than("massB", 10.0)], // unbound -> Inconclusive
        constraint_element_ids: vec![None],
        compile_errors: vec![],
        subrequirements: vec![],
        bindings: vec![],
        binding_specs: vec![],
    });
    let runner = VerificationRunner::new();
    let mut ctx = EvalContext::new();
    ctx.set("massA", Value::Float(50.0)); // 50 < 10 is FALSE -> Fail
    let result = runner.verify(&case, &ctx);
    assert_eq!(
        result.verdict,
        VerdictKind::Fail,
        "a decided Fail must outrank a co-occurring Inconclusive at the case level"
    );
}

// ===========================================================================
// OBL — analysis-case-objective-bound-to-result
// §7.23.2: "the subject of the objective is always bound to the result of the
// analysis case" — the load-bearing asymmetry vs verification cases (whose
// objective subject binds to the CASE subject). The base `Case` objective declares
// `subject subj default Case::result` (Cases.sysml:46), which `AnalysisCase` does
// NOT override; so an analysis case's objective subject IS its result, and the
// verified requirement is checked against the result. CLOSED (FORM-B Tier-1):
// `compile_analysis_case` discovers the objective requirement(s) via the SAME path
// verification cases use and binds the result to the objective subject;
// `AnalysisCaseIR::verify_objective` produces the verdict through the ONE engine
// (`VerificationRunner`). The verdict-flip + negative twins (value-less result →
// Inconclusive) are gated in cases_pipeline.rs::analysis_objective_result_binding_drives_verdict.
// ===========================================================================

#[test]
fn analysis_case_objective_is_bound_to_result() {
    // OBL: analysis-case-objective-bound-to-result
    // VERDICT: CONFORMS — `compile_analysis_case` discovers the objective's verified
    //   requirement and binds the analysis RESULT to the objective subject
    //   (§7.23.2 / Cases.sysml:46 `subject subj default Case::result`, not overridden
    //   by AnalysisCase). `AnalysisCaseIR::verify_objective` routes the verdict through
    //   the one engine (VerificationRunner) — no second verdict path. A value-less
    //   result leaves the subject unbound → honest Inconclusive (§7.24.1).
    let graph = parse(
        r#"
        package AnalysisGate {
            requirement def ResultBelowLimit {
                subject measuredMass : Real;
                require constraint { measuredMass < 10.0 }
            }
            analysis def MassAnalysis {
                return attribute result : Real = 0.3;
                objective { verify ResultBelowLimit; }
            }
        }
    "#,
    );

    let ir = compile_analysis_case("MassAnalysis", &graph)
        .expect("analysis case should compile");

    // The objective's verified requirement is discovered and surfaced as a
    // checkable RequirementCheck — not merely an opaque `objective` string.
    assert!(
        !ir.objective_requirements.is_empty(),
        "spec §7.23.2: the objective's verified requirement must be discovered as a \
         checkable RequirementCheck, but objective_requirements is empty"
    );
    let check = &ir.objective_requirements[0];
    assert!(
        !check.constraints.is_empty(),
        "the verified requirement's constraint (measuredMass < 10.0) must be present"
    );
    // §7.23.2: the objective subject (= analysis result) is bound to the verified
    // requirement's subject. Since the RequirementBinding refactor (6342e1ae) the
    // objective-subject binding is captured as a DEFERRED `binding_specs` entry —
    // a `RequirementBinding::Literal` resolved into the check-time context by the
    // runner — not as an eager `check.bindings` pair (that field is populated only
    // by verify-time result overlay, and is empty at compile/discovery time). So
    // the compile-time surface to assert is `binding_specs`: the result literal 0.3
    // carried under the subject name `measuredMass`.
    assert!(
        check.binding_specs.iter().any(|b| matches!(
            b,
            RequirementBinding::Literal { name, value }
                if name == "measuredMass" && *value == Value::Float(0.3)
        )),
        "the analysis result (0.3) must be bound to the objective subject `measuredMass` \
         as a binding_specs Literal, got binding_specs: {:?}",
        check.binding_specs
    );

    // The verdict is produced through the one engine: result=0.3 satisfies < 10.0 → Pass.
    let verdict = ir.verify_objective(&EvalContext::new()).verdict;
    assert_eq!(
        verdict,
        VerdictKind::Pass,
        "objective subject bound to result=0.3, 0.3<10 must PASS via VerificationRunner"
    );
}

// ===========================================================================
// OBL — verdict-criteria-modeled-explicitly  [SPEC-SILENT default]
// §8.4.20.1: "the criteria for passing must be modeled explicitly." The spec is
// SILENT on what a tool does when NO verdict expression / requirement is present
// at all. cases_pipeline.rs gates the graph-built empty case; here we pin the
// IR-level default and flag it as a tool-defined divergence from "criteria must
// be modeled explicitly" (the tool silently passes rather than refusing).
// ===========================================================================

#[test]
fn case_with_no_criteria_defaults_to_vacuous_pass() {
    // OBL: verdict-criteria-modeled-explicitly
    // VERDICT: CONFORMS — GAP-VER closed: a case with NO modeled requirements
    //   yields Inconclusive (a determination cannot be made), not a vacuous Pass.
    //   SysML §8.4.20.1 (criteria must be modeled explicitly) / §7.24.1.
    let case = VerificationCaseIR {
        id: "empty".into(),
        name: "EmptyCriteria".into(),
        subject: Some("subj".into()),
        setup_actions: vec![],
        requirements: vec![], // NO modeled criteria at all
        sub_cases: vec![],
        // Deliberately ALSO no modeled verdict expression — this test pins
        // the tool default when a case models nothing at all.
        verdict_expression: None,
        bindings: vec![],
    };
    let runner = VerificationRunner::new();
    let result = runner.verify(&case, &EvalContext::new());
    assert_eq!(
        result.verdict,
        VerdictKind::Inconclusive,
        "spec §8.4.20.1/§7.24.1: no modeled criteria -> determination cannot be made -> \
         Inconclusive, not a vacuous Pass"
    );
    assert!(
        result.requirement_results.is_empty(),
        "no requirement results when no criteria are modeled"
    );
}

#[test]
fn requirement_with_empty_constraint_list_passes_vacuously() {
    // OBL: verdict-criteria-modeled-explicitly
    // VERDICT: CONFORMS — GAP-VER closed: a requirement carrying NO constraints
    //   (and no subrequirements) yields Inconclusive, not a vacuous Pass. SysML
    //   §8.4.20.1 / §7.24.1. (Distinct from unmet-assumption vacuous satisfaction.)
    let case = VerificationCaseIR {
        id: "vc".into(),
        name: "EmptyReqConstraints".into(),
        subject: Some("subj".into()),
        setup_actions: vec![],
        requirements: vec![RequirementCheck {
            id: "req".into(),
            source_element_id: None,
            text: Some("no constraints modeled".into()),
            assumptions: vec![],
            constraints: vec![], // no modeled criteria
            constraint_element_ids: vec![],
            compile_errors: vec![],
            subrequirements: vec![],
            bindings: vec![],
            binding_specs: vec![],
        }],
        sub_cases: vec![],
        // No modeled verdict expression — the empty-constraints default is
        // what this test pins.
        verdict_expression: None,
        bindings: vec![],
    };
    let runner = VerificationRunner::new();
    let result = runner.verify(&case, &EvalContext::new());
    assert_eq!(
        result.verdict,
        VerdictKind::Inconclusive,
        "spec §8.4.20.1/§7.24.1: requirement with no modeled constraints -> determination \
         cannot be made -> Inconclusive, not a vacuous Pass"
    );
}

// ===========================================================================
// Matrix summary — self-scans this file and prints verdict counts.
// ===========================================================================

#[test]
fn verification_case_matrix_summary() {
    let src = include_str!("verification_case_spec_conformance.rs");
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
        "VCSC verification/analysis matrix: {} gated obligations — \
         {conforms} CONFORMS, {diverges} DIVERGES, {unimpl} UNIMPLEMENTED",
        verdicts.len()
    );
    assert!(
        verdicts.len() >= 6,
        "expected >=6 verdict-marked gates in the verification/analysis suite"
    );
}
