//! End-to-end pipeline tests: build ModelGraph -> elaborate -> compile -> verify.
//!
//! These tests verify the full verification case pipeline from programmatic
//! graph construction through elaboration, compilation, and verdict generation.

use sysml_core::elaborate::elaborate;
use sysml_core::{Element, ElementKind, ModelGraph, Value};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::cases::{
    compile_analysis_case, compile_verification_case, VerdictKind, VerificationRunner,
};
use sysml_runtime::expressions::EvalContext;
use sysml_runtime::SolverRegistry;

// ---------------------------------------------------------------------------
// Verification case pipeline tests
// ---------------------------------------------------------------------------

/// Build a verification case with a requirement, elaborate, compile, and verify.
///
/// Graph: VerificationCaseDefinition "SpeedCheck"
///   └── RequirementUsage "SpeedReq"
///       with constraint "speed < 100"
#[test]
fn verification_case_elaborate_compile_verify() {
    let mut graph = ModelGraph::new();

    // Create verification case definition
    let vc = Element::new_with_kind(ElementKind::VerificationCaseDefinition)
        .with_name("SpeedCheck")
        .with_prop("subject", "vehicle");
    let vc_id = graph.add_element(vc);

    // Create requirement as child of the verification case
    let req = Element::new_with_kind(ElementKind::RequirementUsage)
        .with_name("SpeedReq")
        .with_owner(vc_id.clone())
        .with_prop("text", "Vehicle speed must be under limit")
        .with_prop("constraint", "speed < 100");
    graph.add_element(req);

    // Elaborate
    let report = elaborate(&mut graph);
    eprintln!("SpeedCheck elaboration: {}", report);

    // Compile
    let case_ir =
        compile_verification_case("SpeedCheck", &graph).expect("Should compile verification case");
    assert_eq!(case_ir.name, "SpeedCheck");
    assert_eq!(case_ir.subject.as_deref(), Some("vehicle"));
    assert_eq!(case_ir.requirements.len(), 1);
    assert_eq!(case_ir.requirements[0].id, "SpeedReq");

    // Verify — should pass when speed < 100
    let runner = VerificationRunner::new();
    let mut ctx = EvalContext::new();
    ctx.set("speed", Value::Float(80.0));

    let result = runner.verify(&case_ir, &ctx);
    assert_eq!(
        result.verdict,
        VerdictKind::Pass,
        "Should pass when speed=80"
    );
    assert_eq!(result.requirement_results.len(), 1);
    assert_eq!(result.requirement_results[0].verdict, VerdictKind::Pass);

    // Verify — should fail when speed >= 100
    ctx.set("speed", Value::Float(150.0));
    let result = runner.verify(&case_ir, &ctx);
    assert_eq!(
        result.verdict,
        VerdictKind::Fail,
        "Should fail when speed=150"
    );
}

/// Verification case with multiple requirements — aggregate verdict.
#[test]
fn verification_case_multiple_requirements() {
    let mut graph = ModelGraph::new();

    let vc =
        Element::new_with_kind(ElementKind::VerificationCaseDefinition).with_name("SafetyCheck");
    let vc_id = graph.add_element(vc);

    // Requirement 1: speed under limit
    let req1 = Element::new_with_kind(ElementKind::RequirementUsage)
        .with_name("SpeedReq")
        .with_owner(vc_id.clone())
        .with_prop("constraint", "speed < 200");
    graph.add_element(req1);

    // Requirement 2: temperature in range
    let req2 = Element::new_with_kind(ElementKind::RequirementUsage)
        .with_name("TempReq")
        .with_owner(vc_id.clone())
        .with_prop("constraint", "temp < 500");
    graph.add_element(req2);

    elaborate(&mut graph);

    let case_ir = compile_verification_case("SafetyCheck", &graph).expect("Should compile case");
    assert_eq!(case_ir.requirements.len(), 2);

    let runner = VerificationRunner::new();

    // Both pass
    let mut ctx = EvalContext::new();
    ctx.set("speed", Value::Float(100.0));
    ctx.set("temp", Value::Float(300.0));
    let result = runner.verify(&case_ir, &ctx);
    assert_eq!(result.verdict, VerdictKind::Pass);

    // One fails
    ctx.set("temp", Value::Float(600.0));
    let result = runner.verify(&case_ir, &ctx);
    assert_eq!(
        result.verdict,
        VerdictKind::Fail,
        "Should fail when one requirement fails"
    );
}

/// Verification case with no modeled requirements is Inconclusive (not a
/// vacuous Pass): pass criteria must be modeled explicitly (SysML §8.4.20.1),
/// so with none a determination cannot be made (§7.24.1).
#[test]
fn verification_case_no_requirements_is_inconclusive() {
    let mut graph = ModelGraph::new();

    let vc =
        Element::new_with_kind(ElementKind::VerificationCaseDefinition).with_name("EmptyCheck");
    graph.add_element(vc);

    elaborate(&mut graph);

    let case_ir =
        compile_verification_case("EmptyCheck", &graph).expect("Should compile empty case");
    assert_eq!(case_ir.requirements.len(), 0);

    let runner = VerificationRunner::new();
    let ctx = EvalContext::new();
    let result = runner.verify(&case_ir, &ctx);
    assert_eq!(
        result.verdict,
        VerdictKind::Inconclusive,
        "empty case (no modeled criteria) -> determination cannot be made -> Inconclusive"
    );
}

/// CONFORMANCE GATE (full-chain aggregation, §2.1a ruling 2026-07-17 —
/// re-blessed from the 2026-07-16 single-hop rule it supersedes).
///
/// Bare `verify reqUsage;` where the usage owns no constraints but is
/// typed by a requirement def MUST evaluate the def's constraints (before
/// the 2026-07-16 rule the bare form stopped at the empty usage and
/// passed vacuously). Under the full-chain closure the aggregation is
/// UNCONDITIONAL: a usage that OWNS constraints evaluates its own AND its
/// def's — KerML `Type::featureMembership` = owned ∪ inherited; the old
/// "owned wins, hop must not fire" expectation modeled the spec's
/// owned-only introspection properties, not the evaluation feature.
#[test]
fn bare_verify_of_typed_usage_evaluates_the_defs_constraints() {
    let source = r#"
        package HopGate {
            requirement def TripReq {
                attribute t;
                require constraint { t < 40 }
                require constraint { t > 0 }
            }
            requirement fastCheck : TripReq;
            requirement ownCheck : TripReq {
                require constraint { 1 > 0 }
            }
            part def Widget;
            verification def BareRef {
                subject s : Widget;
                objective {
                    verify fastCheck;
                }
            }
            verification def OwnedWins {
                subject s : Widget;
                objective {
                    verify ownCheck;
                }
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("hopgate.sysml", source)]);
    elaborate(&mut result.graph);

    let bare = compile_verification_case("BareRef", &result.graph)
        .unwrap_or_else(|d| panic!("compile BareRef: {d:?}"));
    assert_eq!(bare.requirements.len(), 1);
    assert_eq!(
        bare.requirements[0].constraints.len(),
        2,
        "empty typed usage must evaluate its def's two constraints, not pass vacuously"
    );

    let owned = compile_verification_case("OwnedWins", &result.graph)
        .unwrap_or_else(|d| panic!("compile OwnedWins: {d:?}"));
    assert_eq!(owned.requirements.len(), 1);
    assert_eq!(
        owned.requirements[0].constraints.len(),
        3,
        "a usage that owns constraints aggregates its own (1) AND its \
         def's (2) — the closure is unconditional (§2.1a supersedes the \
         owned-wins gate)"
    );
}

// ---------------------------------------------------------------------------
// Parsed .sysml pipeline test
// ---------------------------------------------------------------------------

/// CONFORMANCE GATE (verification subject/objective binding wave, Inc1).
///
/// Exercises the spec/corpus `verify requirement <name> : <Req> { attribute x = v; }`
/// form (the shape a compliance-style verification file uses).
/// This parses to an `ObjectiveMembership → RequirementVerificationMembership →
/// RequirementUsage → FeatureTyping → RequirementDefinition` chain with the
/// redefinition carried as an `AttributeUsage { value }` child of the check-usage.
///
/// Spec basis: a VerificationCase evaluates its objective RequirementCheck against
/// the verified requirement and returns a VerdictKind (VerificationCases.sysml:21-22,
/// 27, 70-79). The verdict MUST flip on the redefinition binding, and an unwired
/// (undefined) parameter MUST be Inconclusive — not a silent vacuous pass.
#[test]
fn verify_requirement_redefinition_binding_drives_verdict() {
    let source = r#"
        package L2Gate {
            requirement def ThresholdReq {
                attribute limit;
                require constraint { limit > 7 }
            }
            part def Widget;
            verification def FailCase {
                subject s : Widget;
                objective {
                    verify requirement check : ThresholdReq { attribute limit = 5; }
                }
            }
            verification def PassCase {
                subject s : Widget;
                objective {
                    verify requirement check : ThresholdReq { attribute limit = 20; }
                }
            }
            verification def UnwiredCase {
                subject s : Widget;
                objective {
                    verify requirement check : ThresholdReq;
                }
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("l2gate.sysml", source)]);
    elaborate(&mut result.graph);

    let runner = VerificationRunner::new();
    let run = |case: &str| {
        let ir = compile_verification_case(case, &result.graph)
            .unwrap_or_else(|d| panic!("compile {case}: {d:?}"));
        eprintln!(
            "{case}: requirements={}, ids={:?}",
            ir.requirements.len(),
            ir.requirements
                .iter()
                .map(|r| (&r.id, r.constraints.len()))
                .collect::<Vec<_>>()
        );
        let vr = runner.verify(&ir, &EvalContext::new());
        eprintln!("  -> verdict={}", vr.verdict);
        vr.verdict
    };

    assert_eq!(run("FailCase"), VerdictKind::Fail, "limit=5, 5>7 must FAIL");
    assert_eq!(
        run("PassCase"),
        VerdictKind::Pass,
        "limit=20, 20>7 must PASS"
    );
    assert_eq!(
        run("UnwiredCase"),
        VerdictKind::Inconclusive,
        "no redefinition -> limit undefined -> Inconclusive, NOT silent pass"
    );
}

/// CONFORMANCE GATE (Inc1b): reference forms — `require constraint : Def` and the
/// bare `verify Req;` clause.
///
/// `require constraint : LimitOk;` carries no inline body — the constraint lives in
/// the referenced `constraint def`. The ast-builder now stamps the reference
/// (`referencedConstraint`) and the runner follows it. The bare `verify Req;` form
/// (`RequirementVerificationMembership`, target by `verifiedRequirement`) is also
/// discovered. With no value bound, the referenced constraint must be Inconclusive
/// — never a vacuous pass that silently skips the (now-found) constraint.
#[test]
fn referenced_constraint_and_bare_verify_drive_verdict() {
    let source = r#"
        package L1bGate {
            constraint def LimitOk { limit > 7 }
            requirement def ThresholdReq {
                attribute limit;
                require constraint : LimitOk;
            }
            part def Widget;
            verification def FailCase {
                subject s : Widget;
                objective {
                    verify requirement check : ThresholdReq { attribute limit = 5; }
                }
            }
            verification def PassCase {
                subject s : Widget;
                objective {
                    verify requirement check : ThresholdReq { attribute limit = 20; }
                }
            }
            verification def BareUnboundCase {
                subject s : Widget;
                objective {
                    verify ThresholdReq;
                }
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("l1bgate.sysml", source)]);
    // Reference forms mint real ReferenceSubsetting / FeatureTyping relationships
    // whose targets `referenced_constraint_target` reads AFTER resolution — the
    // production verification graph is always resolved+elaborated (ide-db
    // `elaborate_workspace_with_library`), so resolve here too.
    sysml_core::resolution::resolve_references(&mut result.graph);
    elaborate(&mut result.graph);

    let runner = VerificationRunner::new();
    let run = |case: &str| {
        let ir = compile_verification_case(case, &result.graph)
            .unwrap_or_else(|d| panic!("compile {case}: {d:?}"));
        eprintln!(
            "{case}: requirements={}, constraints={:?}",
            ir.requirements.len(),
            ir.requirements
                .iter()
                .map(|r| r.constraints.len())
                .collect::<Vec<_>>()
        );
        runner.verify(&ir, &EvalContext::new()).verdict
    };

    assert_eq!(
        run("FailCase"),
        VerdictKind::Fail,
        "limit=5 redefined, referenced constraint `limit > 7` must FAIL"
    );
    assert_eq!(
        run("PassCase"),
        VerdictKind::Pass,
        "limit=20 redefined, referenced constraint `limit > 7` must PASS"
    );
    assert_eq!(
        run("BareUnboundCase"),
        VerdictKind::Inconclusive,
        "bare `verify ThresholdReq` with no bound value -> referenced constraint \
         is found but `limit` is undefined -> Inconclusive, not a vacuous pass"
    );
}

/// CONFORMANCE GATE (Inc1c): subject binding (layer 1).
///
/// A requirement's constraints are written in terms of its subject parameter
/// (`subject w : Widget; require constraint { w.mass < 10 }`). Per spec the
/// verification case's subject is identified with the requirement's subject
/// (VerificationCases.sysml:25) and value equality flows only through that
/// binding — there is no implicit name-matching. Binding the subject's declared
/// attribute values (`w.mass`) MUST drive the verdict: the same constraint flips
/// PASS↔FAIL purely on the subject type's attribute value, and a value-less
/// subject type stays Inconclusive (never a silent vacuous pass).
#[test]
fn subject_attribute_binding_drives_verdict() {
    let source = r#"
        package SubjGate {
            part def Widget { attribute mass : Real = 5.0; }
            part def HeavyWidget { attribute mass : Real = 50.0; }
            part def BareWidget;
            requirement def MassReq {
                subject w : Widget;
                require constraint { w.mass < 10.0 }
            }
            requirement def HeavyReq {
                subject w : HeavyWidget;
                require constraint { w.mass < 10.0 }
            }
            requirement def BareReq {
                subject w : BareWidget;
                require constraint { w.mass < 10.0 }
            }
            verification def MassPassCase {
                subject w : Widget;
                objective { verify MassReq; }
            }
            verification def MassFailCase {
                subject w : HeavyWidget;
                objective { verify HeavyReq; }
            }
            verification def MassUnboundCase {
                subject w : BareWidget;
                objective { verify BareReq; }
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("subjgate.sysml", source)]);
    elaborate(&mut result.graph);

    let runner = VerificationRunner::new();
    let run = |case: &str| {
        let ir = compile_verification_case(case, &result.graph)
            .unwrap_or_else(|d| panic!("compile {case}: {d:?}"));
        eprintln!(
            "{case}: requirements={}, bindings={:?}",
            ir.requirements.len(),
            ir.requirements
                .iter()
                .map(|r| &r.bindings)
                .collect::<Vec<_>>()
        );
        runner.verify(&ir, &EvalContext::new()).verdict
    };

    assert_eq!(
        run("MassPassCase"),
        VerdictKind::Pass,
        "w.mass=5.0 bound from subject Widget, 5<10 must PASS"
    );
    assert_eq!(
        run("MassFailCase"),
        VerdictKind::Fail,
        "w.mass=50.0 bound from subject HeavyWidget, 50<10 must FAIL"
    );
    assert_eq!(
        run("MassUnboundCase"),
        VerdictKind::Inconclusive,
        "BareWidget has no mass value -> w.mass undefined -> Inconclusive, not a vacuous pass"
    );
}

/// Inc2a gate: a verify clause may bind a requirement attribute to the value of a
/// referenced feature rather than a literal — `attribute :>> w.mass = run.result;`
/// (a FeatureValue whose RHS is a FeatureReferenceExpression, ≡ a BindingConnector,
/// Kerml-Vocab.ttl:42-44). Per spec the value flows ONLY through the model's
/// explicit `=` bindings, so an analysis case usage that declares its result with a
/// literal (`return result = 0.3;`) is the value source — exactly how a sim/analysis
/// output reaches the verdict. The verdict MUST flip purely on that referenced
/// result, and an absent/value-less binding stays Inconclusive (never vacuous).
#[test]
fn feature_reference_binding_drives_verdict() {
    let source = r#"
        package FeatureRefGate {
            part def Widget { attribute mass : Real; }
            requirement def MassReq {
                subject w : Widget;
                require constraint { w.mass < 10.0 }
            }
            analysis def MassAnalysis { return attribute result : Real; }
            verification def PassCase {
                subject testWidget : Widget;
                analysis run : MassAnalysis { return result = 0.3; }
                objective {
                    verify requirement checkMass : MassReq {
                        attribute :>> w.mass = run.result;
                    }
                }
            }
            verification def FailCase {
                subject testWidget : Widget;
                analysis run : MassAnalysis { return result = 50.0; }
                objective {
                    verify requirement checkMass : MassReq {
                        attribute :>> w.mass = run.result;
                    }
                }
            }
            verification def UnboundCase {
                subject testWidget : Widget;
                analysis run : MassAnalysis { return attribute result : Real; }
                objective {
                    verify requirement checkMass : MassReq {
                        attribute :>> w.mass = run.result;
                    }
                }
            }
            verification def NoBindingCase {
                subject testWidget : Widget;
                analysis run : MassAnalysis { return result = 0.3; }
                objective {
                    verify requirement checkMass : MassReq;
                }
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("featurerefgate.sysml", source)]);
    elaborate(&mut result.graph);

    let runner = VerificationRunner::new();
    let run = |case: &str| {
        let ir = compile_verification_case(case, &result.graph)
            .unwrap_or_else(|d| panic!("compile {case}: {d:?}"));
        runner.verify(&ir, &EvalContext::new()).verdict
    };

    assert_eq!(
        run("PassCase"),
        VerdictKind::Pass,
        "w.mass bound to run.result=0.3 via feature reference, 0.3<10 must PASS"
    );
    assert_eq!(
        run("FailCase"),
        VerdictKind::Fail,
        "w.mass bound to run.result=50.0 via feature reference, 50<10 must FAIL"
    );
    assert_eq!(
        run("UnboundCase"),
        VerdictKind::Inconclusive,
        "run.result carries no literal -> w.mass unresolved -> Inconclusive, not vacuous"
    );
    assert_eq!(
        run("NoBindingCase"),
        VerdictKind::Inconclusive,
        "no binding clause -> w.mass undefined -> Inconclusive, not vacuous"
    );
}

/// Inc2 (subject-under-test) gate: a verification case may bind its subject to a
/// concrete occurrence — `subject testW : Widget = lightWidget;` (spec §7.24 /
/// Annex A:1094 `subject = vehicle_b`). Per VerificationCases.sysml:25 the verified
/// requirement's subject is identified with the case subject, so the occurrence's
/// attribute values (not the requirement subject TYPE's defaults) MUST drive the
/// verdict. The same `w.mass < 10.0` flips PASS↔FAIL purely on which occurrence the
/// case subject is bound to, and a subject with no bound occurrence and no type
/// default stays Inconclusive (never a vacuous pass).
#[test]
fn subject_occurrence_binding_drives_verdict() {
    let source = r#"
        package SubjOccGate {
            part def Widget { attribute mass : Real; }
            part lightWidget : Widget { attribute :>> mass = 3.0; }
            part heavyWidget : Widget { attribute :>> mass = 80.0; }
            requirement def MassReq {
                subject w : Widget;
                require constraint { w.mass < 10.0 }
            }
            verification def PassCase {
                subject testW : Widget = lightWidget;
                objective { verify requirement c : MassReq; }
            }
            verification def FailCase {
                subject testW : Widget = heavyWidget;
                objective { verify requirement c : MassReq; }
            }
            verification def UnboundCase {
                subject testW : Widget;
                objective { verify requirement c : MassReq; }
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("subjoccgate.sysml", source)]);
    elaborate(&mut result.graph);

    let runner = VerificationRunner::new();
    let run = |case: &str| {
        let ir = compile_verification_case(case, &result.graph)
            .unwrap_or_else(|d| panic!("compile {case}: {d:?}"));
        runner.verify(&ir, &EvalContext::new()).verdict
    };

    assert_eq!(
        run("PassCase"),
        VerdictKind::Pass,
        "case subject bound to lightWidget(mass=3.0), 3<10 must PASS"
    );
    assert_eq!(
        run("FailCase"),
        VerdictKind::Fail,
        "case subject bound to heavyWidget(mass=80.0), 80<10 must FAIL"
    );
    assert_eq!(
        run("UnboundCase"),
        VerdictKind::Inconclusive,
        "case subject typed only, no occurrence + no type default -> Inconclusive, not vacuous"
    );
}

/// FORM-B Tier-1 (analysis-case objective→result) gate. §7.23.2: "the subject of
/// the objective is always bound to the result of the analysis case." The base
/// `Case` objective declares `subject subj default Case::result` (Cases.sysml:46),
/// which `AnalysisCase` does NOT override — so an analysis case's objective subject
/// is its RESULT, exactly as a verification case's objective subject is its CASE
/// SUBJECT (VerificationCases.sysml:25). They are the same objective→verdict pipeline
/// differing only in the subject-value source.
///
/// Here the verified requirement's `measuredMass < 10.0` flips PASS↔FAIL purely on
/// the analysis result literal bound to its subject, and a value-less result leaves
/// the subject unbound → Inconclusive (never a vacuous pass). The verdict is produced
/// through the ONE engine via `AnalysisCaseIR::verify_objective`.
#[test]
fn analysis_objective_result_binding_drives_verdict() {
    let source = r#"
        package AnalysisObjectiveGate {
            requirement def ResultBelowLimit {
                subject measuredMass : Real;
                require constraint { measuredMass < 10.0 }
            }
            analysis def PassAnalysis {
                return attribute result : Real = 0.3;
                objective { verify ResultBelowLimit; }
            }
            analysis def FailAnalysis {
                return attribute result : Real = 50.0;
                objective { verify ResultBelowLimit; }
            }
            analysis def UnboundAnalysis {
                return attribute result : Real;
                objective { verify ResultBelowLimit; }
            }
            analysis def NoObjectiveAnalysis {
                return attribute result : Real = 0.3;
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("analysisobjgate.sysml", source)]);
    elaborate(&mut result.graph);

    let run = |case: &str| {
        let ir = compile_analysis_case(case, &result.graph)
            .unwrap_or_else(|d| panic!("compile {case}: {d:?}"));
        ir.verify_objective(&EvalContext::new()).verdict
    };

    assert_eq!(
        run("PassAnalysis"),
        VerdictKind::Pass,
        "objective subject bound to result=0.3 (Cases.sysml:46), 0.3<10 must PASS"
    );
    assert_eq!(
        run("FailAnalysis"),
        VerdictKind::Fail,
        "objective subject bound to result=50.0, 50<10 must FAIL"
    );
    assert_eq!(
        run("UnboundAnalysis"),
        VerdictKind::Inconclusive,
        "value-less result -> objective subject unbound -> Inconclusive, not vacuous"
    );
    assert_eq!(
        run("NoObjectiveAnalysis"),
        VerdictKind::Inconclusive,
        "no objective requirement to check -> determination cannot be made -> Inconclusive"
    );
}

/// FORM-B Tier-2 (expression/calc result) gate. The analysis result may be computed
/// by an expression over the case's inputs — `return attribute result = base + 2.0`
/// (§8.3.23.2 ResultExpressionMembership). The expression is evaluated statically
/// against the model-declared input defaults (design plan §4.2), and that computed
/// result drives the objective verdict exactly as a literal does. The same
/// `measuredMass < 10.0` flips PASS↔FAIL on the computed result; an input with no
/// model-declared default leaves the result unresolved → Inconclusive (never
/// fabricated). Value flows only through model `=` (B1).
#[test]
fn analysis_objective_expression_result_drives_verdict() {
    let source = r#"
        package AnalysisExprGate {
            requirement def ResultBelowLimit {
                subject measuredMass : Real;
                require constraint { measuredMass < 10.0 }
            }
            analysis def PassExpr {
                in attribute base : Real = 3.0;
                return attribute result : Real = base + 2.0;
                objective { verify ResultBelowLimit; }
            }
            analysis def FailExpr {
                in attribute base : Real = 30.0;
                return attribute result : Real = base + 2.0;
                objective { verify ResultBelowLimit; }
            }
            analysis def UnboundExpr {
                in attribute base : Real;
                return attribute result : Real = base + 2.0;
                objective { verify ResultBelowLimit; }
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("analysisexprgate.sysml", source)]);
    elaborate(&mut result.graph);

    let run = |case: &str| {
        let ir = compile_analysis_case(case, &result.graph)
            .unwrap_or_else(|d| panic!("compile {case}: {d:?}"));
        ir.verify_objective(&EvalContext::new()).verdict
    };

    assert_eq!(
        run("PassExpr"),
        VerdictKind::Pass,
        "result = base(3.0)+2.0 = 5.0 bound to objective subject, 5<10 must PASS"
    );
    assert_eq!(
        run("FailExpr"),
        VerdictKind::Fail,
        "result = base(30.0)+2.0 = 32.0, 32<10 must FAIL"
    );
    assert_eq!(
        run("UnboundExpr"),
        VerdictKind::Inconclusive,
        "input `base` has no model default -> result expression unresolved -> Inconclusive"
    );
}

/// FORM-B Tier-3 (executed result) gate. §7.23.1 enumerates "executed" result
/// sources — analysis actions, an external solver, ODE integration — whose result is
/// only known AFTER running the analysis. Here a `constraint { x + 5.0 == target }`
/// is SOLVED by the analysis case's own solver to produce `x`, and `result = x` drives
/// the objective verdict. `AnalysisCaseIR::run_and_verify` runs the solver, binds the
/// executed result to the objective subject, and verdicts through the ONE engine.
///
/// CRITICAL (proves it does what T2 cannot): the SAME case under `verify_objective`
/// (compile-time / static, no execution) is Inconclusive — `x` is solver-found, not a
/// model-declared default — but under `run_and_verify` the executed result yields a
/// decisive verdict that flips PASS↔FAIL on the solved value.
#[test]
fn analysis_objective_executed_result_drives_verdict() {
    let source = r#"
        package AnalysisExecGate {
            requirement def ResultBelowLimit {
                subject measuredMass : Real;
                require constraint { measuredMass < 10.0 }
            }
            analysis def PassExec {
                @ToolExecution { attribute toolName = "builtin:bisection"; }
                in attribute target : Real = 8.0;
                attribute x : Real;
                constraint { x - (target - 5.0) }
                return attribute result : Real = x;
                objective { verify ResultBelowLimit; }
            }
            analysis def FailExec {
                @ToolExecution { attribute toolName = "builtin:bisection"; }
                in attribute target : Real = 20.0;
                attribute x : Real;
                constraint { x - (target - 5.0) }
                return attribute result : Real = x;
                objective { verify ResultBelowLimit; }
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("analysisexecgate.sysml", source)]);
    elaborate(&mut result.graph);
    let registry = SolverRegistry::with_builtins();

    let pass_ir = compile_analysis_case("PassExec", &result.graph)
        .unwrap_or_else(|d| panic!("compile PassExec: {d:?}"));

    // T2 boundary: without executing the solver, `x` is unknown -> result unresolved ->
    // the objective subject is unbound -> Inconclusive. This is what run_and_verify must
    // improve on (proves Tier-3 is doing something the static tiers cannot).
    assert_eq!(
        pass_ir.verify_objective(&EvalContext::new()).verdict,
        VerdictKind::Inconclusive,
        "static (no execution): solver-found `x` is unknown -> Inconclusive"
    );

    // Tier-3: run the solver -> x = target-5 -> result = x -> verdict.
    assert_eq!(
        pass_ir
            .run_and_verify(&registry, &EvalContext::new())
            .verdict,
        VerdictKind::Pass,
        "solver finds x = 8-5 = 3, result = 3 < 10 must PASS"
    );

    let fail_ir = compile_analysis_case("FailExec", &result.graph)
        .unwrap_or_else(|d| panic!("compile FailExec: {d:?}"));
    assert_eq!(
        fail_ir
            .run_and_verify(&registry, &EvalContext::new())
            .verdict,
        VerdictKind::Fail,
        "solver finds x = 20-5 = 15, result = 15, 15 < 10 must FAIL"
    );

    // No-double-solve seam: a caller that already solved (e.g. `sysml.analysis.run`,
    // which needs the solver outputs for its own result surface) verifies the
    // SAME already-solved result via `verify_solved_objective` and gets the same
    // verdict `run_and_verify` produces — one verdict path, one solve.
    let solved = pass_ir
        .execute(&registry, &EvalContext::new())
        .expect("PassExec solves");
    let solved_objective = pass_ir.verify_solved_objective(&solved, &EvalContext::new());
    assert_eq!(
        solved_objective.result.verdict,
        VerdictKind::Pass,
        "verify_solved_objective over the already-solved result matches run_and_verify"
    );
    assert_eq!(
        solved_objective.case.requirements.len(),
        pass_ir.objective_requirements.len(),
        "the returned case exposes the objective requirements for verdict projection"
    );
}

/// Parse a .sysml string containing a verification case, elaborate, compile, and verify.
#[test]
fn parsed_sysml_verification_case_pipeline() {
    let source = r#"
        package SafetyVerification {
            verification def SpeedCheck {
                subject vehicle;
                requirement SpeedReq {
                    doc /* Speed must be under 200 */
                }
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("test.sysml", source)]);

    eprintln!(
        "Parsed verification case: {} elements, {} diagnostics",
        result.graph.element_count(),
        result.diagnostics.len()
    );

    // Elaborate
    let report = elaborate(&mut result.graph);
    eprintln!("Verification case elaboration: {}", report);

    // Check for verification case definitions
    let vc_defs: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::VerificationCaseDefinition)
        .collect();

    eprintln!("VerificationCaseDefinition elements: {}", vc_defs.len());

    if !vc_defs.is_empty() {
        assert_eq!(
            vc_defs[0].name.as_deref(),
            Some("SpeedCheck"),
            "VerificationCaseDefinition should be named 'SpeedCheck'"
        );

        // Try compilation
        match compile_verification_case("SpeedCheck", &result.graph) {
            Ok(case_ir) => {
                eprintln!(
                    "Compiled: name={}, requirements={}, subject={:?}",
                    case_ir.name,
                    case_ir.requirements.len(),
                    case_ir.subject
                );

                // Verify
                let runner = VerificationRunner::new();
                let ctx = EvalContext::new();
                let vr = runner.verify(&case_ir, &ctx);
                eprintln!("Verdict: {}", vr.verdict);
            }
            Err(diags) => {
                eprintln!("Compilation diagnostics:");
                for d in &diags {
                    eprintln!("  - {}", d.message);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Full-chain aggregation gates (§2.1a ruling 2026-07-17)
// ---------------------------------------------------------------------------

/// CONFORMANCE GATE: def specialization (`requirement def A :> B`) binds B's
/// constraints transitively — the chain is typing + specialization, not one
/// hop. Verifying a usage of the MOST derived def must evaluate the whole
/// ancestry's constraints as one conjunction.
#[test]
fn def_specialization_chain_aggregates_transitively() {
    let source = r#"
        package ChainGate {
            requirement def Root {
                attribute t;
                require constraint { t > 0 }
            }
            requirement def Mid :> Root {
                require constraint { t < 100 }
            }
            requirement def Leaf :> Mid {
                require constraint { t < 40 }
            }
            requirement leafCheck : Leaf;
            part def Widget;
            verification def ChainCase {
                subject s : Widget;
                objective {
                    verify leafCheck;
                }
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("chaingate.sysml", source)]);
    elaborate(&mut result.graph);

    let case = compile_verification_case("ChainCase", &result.graph)
        .unwrap_or_else(|d| panic!("compile ChainCase: {d:?}"));
    assert_eq!(case.requirements.len(), 1);
    assert_eq!(
        case.requirements[0].constraints.len(),
        3,
        "Leaf inherits Mid's and Root's constraints through the \
         specialization chain (t>0, t<100, t<40)"
    );
}

/// CONFORMANCE GATE (§2.1a ruling (c)): `require <requirementName>;`
/// referencing another REQUIREMENT binds it as ONE nested obligation — the
/// referenced requirement's own aggregate result contributes as a single
/// sub-check (never a flattening of its constraint list), and its verdict
/// gates the referencing requirement's. Before this ruling the reference
/// surfaced in the workbench contract but contributed NOTHING to the
/// verdict (the document-idiom demo's `require emcCompliance;` dead spot).
#[test]
fn requirement_reference_binds_as_nested_obligation() {
    let source = r#"
        package RefGate {
            part def Widget {
                attribute clearance = 3.0;
            }
            requirement emcCompliance {
                subject w : Widget;
                require constraint { w.clearance > 5.0 }
            }
            requirement systemReq {
                subject w : Widget;
                require constraint { 1 < 2 }
                require emcCompliance;
            }
            verification def RefCase {
                subject s : Widget;
                objective {
                    verify systemReq;
                }
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("refgate.sysml", source)]);
    // Resolve the bare-name ReferenceSubsetting (`require emcCompliance;`) before
    // the runtime reads its resolved target (matches the production graph).
    sysml_core::resolution::resolve_references(&mut result.graph);
    elaborate(&mut result.graph);

    let case = compile_verification_case("RefCase", &result.graph)
        .unwrap_or_else(|d| panic!("compile RefCase: {d:?}"));
    assert_eq!(case.requirements.len(), 1);
    let check = &case.requirements[0];
    assert_eq!(check.constraints.len(), 1, "own constraint compiles");
    assert_eq!(
        check.subrequirements.len(),
        1,
        "the referenced requirement is ONE nested obligation, not a \
         flattened constraint list"
    );
    assert_eq!(check.subrequirements[0].id, "emcCompliance");
    assert_eq!(
        check.subrequirements[0].constraints.len(),
        1,
        "the nested obligation carries the referenced requirement's own \
         constraint"
    );

    // And it GATES the verdict: clearance=3.0 fails `> 5.0`, so the
    // referencing requirement must fail even though its own constraint
    // passes — before the ruling this passed vacuously.
    let runner = VerificationRunner::new();
    let ctx = EvalContext::new();
    let outcome = runner.verify(&case, &ctx);
    assert_eq!(
        outcome.verdict,
        VerdictKind::Fail,
        "nested reference obligation must gate the parent verdict"
    );
}

/// CONFORMANCE GATE: reference cycles (`A requires B; B requires A`)
/// terminate as an honest compile error on the check — never a hang, never
/// a silent skip.
#[test]
fn cyclic_requirement_references_terminate_with_compile_error() {
    let source = r#"
        package CycleGate {
            part def Widget;
            requirement reqA {
                subject w : Widget;
                require constraint { 1 < 2 }
                require reqB;
            }
            requirement reqB {
                subject w : Widget;
                require reqA;
            }
            verification def CycleCase {
                subject s : Widget;
                objective {
                    verify reqA;
                }
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("cyclegate.sysml", source)]);
    // Resolve the reference-form cycle before the runtime follows it.
    sysml_core::resolution::resolve_references(&mut result.graph);
    elaborate(&mut result.graph);

    let case = compile_verification_case("CycleCase", &result.graph)
        .unwrap_or_else(|d| panic!("compile CycleCase: {d:?}"));
    let check = &case.requirements[0];
    assert_eq!(check.subrequirements.len(), 1, "reqB nests under reqA");
    assert!(
        check.subrequirements[0]
            .compile_errors
            .iter()
            .any(|e| e.contains("cyclic requirement reference")),
        "the back-reference to reqA must surface as an honest compile \
         error, got: {:?}",
        check.subrequirements[0].compile_errors
    );
}

/// CONFORMANCE GATE (§2.1a ruling (b) — redefinition suppression, with the
/// grammar fix that makes it REACHABLE): `require constraint foo :>> Base::foo`
/// must (1) LOWER to a real `Redefinition` element owned by the membership —
/// grammar acceptance alone proves nothing; before the fix the clause
/// shattered into error-recovery debris — and (2) SUPPRESS the redefined
/// inherited constraint in the effective set (KerML
/// `Type::removeRedefinedFeatures`): the redefining member replaces it,
/// never both.
#[test]
fn requirement_constraint_redefinition_lowers_and_suppresses() {
    let source = r#"
        package RedefGate {
            requirement def Base {
                attribute m;
                require constraint maxMass { m < 10 }
                require constraint other { m > 0 }
            }
            requirement def Derived :> Base {
                require constraint maxMass :>> Base::maxMass { m < 5 }
            }
            requirement derivedCheck : Derived;
            part def Widget;
            verification def RedefCase {
                subject s : Widget;
                objective {
                    verify derivedCheck;
                }
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("redefgate.sysml", source)]);
    elaborate(&mut result.graph);

    // (1) The lowering: a real Redefinition element, owned by Derived's
    // maxMass membership, carrying the redefined-feature name — the exact
    // shape `redefined_feature_name` (the shared sysml-core reader) reads.
    let derived_id = result
        .graph
        .elements
        .values()
        .find(|e| {
            e.name.as_deref() == Some("Derived")
                && e.kind == ElementKind::RequirementDefinition
        })
        .expect("Derived def")
        .id
        .clone();
    let membership = result
        .graph
        .children_of(&derived_id)
        .find(|c| {
            c.kind == ElementKind::RequirementConstraintMembership
                && c.name.as_deref() == Some("maxMass")
        })
        .expect("Derived's maxMass membership");
    let redefinition = result
        .graph
        .children_of(&membership.id)
        .find(|c| c.kind == ElementKind::Redefinition)
        .expect(
            "the :>> clause must lower to a Redefinition ELEMENT on the \
             membership — not subsetting, not phantom debris",
        );
    let target = redefinition
        .get_prop("unresolved_redefinedFeature")
        .and_then(|v| v.as_str())
        .expect("Redefinition carries the redefined-feature name");
    assert!(
        target.ends_with("maxMass"),
        "redefined feature should be Base::maxMass, got {target}"
    );

    // (2) The suppression: Derived's effective set = its redefining maxMass
    // (m < 5) + Base's other (m > 0). Base's maxMass (m < 10) is REPLACED.
    let case = compile_verification_case("RedefCase", &result.graph)
        .unwrap_or_else(|d| panic!("compile RedefCase: {d:?}"));
    assert_eq!(case.requirements.len(), 1);
    assert_eq!(
        case.requirements[0].constraints.len(),
        2,
        "redefining member + un-redefined inherited member; the redefined \
         Base::maxMass must be suppressed, never double-counted"
    );
}

/// CONFORMANCE GATE (full-chain completion): a requirement's OWN declared
/// attribute values (`attribute :>> gap = 8.0;` on a template
/// instantiation, `attribute margin = 2.5;` on a plain requirement) bind
/// its check context — without this every instantiation evaluated
/// Inconclusive because the binding never left the model. Nearest
/// declaration wins along the chain.
#[test]
fn requirement_own_attribute_values_bind_the_check() {
    let source = r#"
        package BindGate {
            requirement def Rule {
                attribute gap;
                require constraint minGap { gap >= 4.0 }
            }
            requirement wideEnough : Rule {
                attribute :>> gap = 8.0;
            }
            requirement tooNarrow : Rule {
                attribute :>> gap = 2.0;
            }
            part def Widget;
            verification def PassCase {
                subject s : Widget;
                objective { verify wideEnough; }
            }
            verification def FailCase {
                subject s : Widget;
                objective { verify tooNarrow; }
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("bindgate.sysml", source)]);
    elaborate(&mut result.graph);

    let runner = VerificationRunner::new();
    let ctx = EvalContext::new();

    let pass = compile_verification_case("PassCase", &result.graph)
        .unwrap_or_else(|d| panic!("compile PassCase: {d:?}"));
    assert_eq!(
        runner.verify(&pass, &ctx).verdict,
        VerdictKind::Pass,
        "gap=8.0 declared on the usage must bind and satisfy gap >= 4.0"
    );

    let fail = compile_verification_case("FailCase", &result.graph)
        .unwrap_or_else(|d| panic!("compile FailCase: {d:?}"));
    assert_eq!(
        runner.verify(&fail, &ctx).verdict,
        VerdictKind::Fail,
        "gap=2.0 declared on the usage must bind and FAIL gap >= 4.0 — \
         never an Inconclusive from an unbound gap"
    );
}

/// v2 UNIFICATION GATE (workbench design §7.1): assume/require bodies
/// compile AST-FIRST — the evaluator walks the structured expression
/// subtree the parser now mints, never a runtime re-parse of the verbatim
/// text. The body below is the differential: the real grammar parses
/// `100.0 - 60.0 - 30.0` left-associatively (= 10.0 → Pass); the legacy
/// string re-parser split at the FIRST `-` (right-associative, = 70.0 →
/// Fail). A Fail here means the evaluator regressed to text re-parsing.
#[test]
fn requirement_constraint_bodies_compile_ast_first() {
    let source = r#"
        package AstGate {
            part def Widget;
            requirement leftAssoc {
                subject w : Widget;
                require constraint { 100.0 - 60.0 - 30.0 == 10.0 }
            }
            verification def AstCase {
                subject s : Widget;
                objective { verify leftAssoc; }
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("astgate.sysml", source)]);
    elaborate(&mut result.graph);

    let runner = VerificationRunner::new();
    let ctx = EvalContext::new();

    let case = compile_verification_case("AstCase", &result.graph)
        .unwrap_or_else(|d| panic!("compile AstCase: {d:?}"));
    assert_eq!(
        runner.verify(&case, &ctx).verdict,
        VerdictKind::Pass,
        "left-associative subtraction must evaluate via the structured AST \
         (10.0); a Fail means the body was re-parsed from text (70.0)"
    );
}

/// §7.1(b)'s fail-hard guard survives the collector-slot skip (steward
/// ruling 2026-07-17): a member that CARRIES a constraint body which
/// fails to compile stays an honest Error — the walker skip applies
/// only to bodyless collector slots, never to members with content.
#[test]
fn failing_constraint_body_still_errors_after_collector_skip() {
    let mut graph = ModelGraph::new();
    let vc =
        Element::new_with_kind(ElementKind::VerificationCaseDefinition).with_name("GuardCase");
    let vc_id = graph.add_element(vc);
    let req = Element::new_with_kind(ElementKind::RequirementUsage)
        .with_name("BadReq")
        .with_owner(vc_id.clone());
    let req_id = graph.add_element(req);
    let membership = Element::new_with_kind(ElementKind::RequirementConstraintMembership)
        .with_name("bad")
        .with_owner(req_id.clone())
        .with_prop("constraint", "@@@ not an expression @@@");
    graph.add_element(membership);

    elaborate(&mut graph);

    let case_ir = compile_verification_case("GuardCase", &graph).expect("Should compile case");
    let runner = VerificationRunner::new();
    let result = runner.verify(&case_ir, &EvalContext::new());
    assert_eq!(
        result.verdict,
        VerdictKind::Error,
        "a carried body that fails to compile must stay an honest Error"
    );
}

/// A reference form naming something that resolves to NEITHER a constraint
/// nor a requirement (`require ghostConstraint;` with no such element) must
/// surface as an honest Error verdict — never silently contribute nothing
/// (fail-hard gap filed 2026-07-17).
#[test]
fn dangling_constraint_reference_is_an_honest_error() {
    let mut graph = ModelGraph::new();
    let vc =
        Element::new_with_kind(ElementKind::VerificationCaseDefinition).with_name("DanglingCase");
    let vc_id = graph.add_element(vc);
    let req = Element::new_with_kind(ElementKind::RequirementUsage)
        .with_name("DanglingReq")
        .with_owner(vc_id.clone());
    let req_id = graph.add_element(req);
    // Spec shape (SysML.xtext:2061-2064): the membership owns a ConstraintUsage
    // which owns a ReferenceSubsetting to the (here nonexistent) target. Leaving
    // `referencedFeature` unresolved (only the parse-time
    // `unresolved_referencedFeature`) models the dangling reference —
    // `referenced_constraint_target` yields nothing, `referenced_constraint_ref_name`
    // still names it for the diagnostic.
    let membership = Element::new_with_kind(ElementKind::RequirementConstraintMembership)
        .with_name("dangling")
        .with_owner(req_id.clone())
        .with_prop("role", "Require");
    let membership_id = graph.add_element(membership);
    let owned_constraint = Element::new_with_kind(ElementKind::ConstraintUsage)
        .with_owner(membership_id.clone());
    let owned_constraint_id = graph.add_element(owned_constraint);
    let ref_sub = Element::new_with_kind(ElementKind::ReferenceSubsetting)
        .with_owner(owned_constraint_id.clone())
        .with_prop("referencingFeature", Value::Ref(owned_constraint_id.clone()))
        .with_prop("unresolved_referencedFeature", "ghostConstraint");
    graph.add_element(ref_sub);

    elaborate(&mut graph);

    let case_ir = compile_verification_case("DanglingCase", &graph).expect("Should compile case");
    let runner = VerificationRunner::new();
    let result = runner.verify(&case_ir, &EvalContext::new());
    assert_eq!(
        result.verdict,
        VerdictKind::Error,
        "a dangling constraint reference must be an honest Error, not a silent skip"
    );
    assert!(
        result
            .requirement_results
            .iter()
            .any(|r| r.message.contains("ghostConstraint")),
        "the error must name the dangling reference; got: {:?}",
        result
            .requirement_results
            .iter()
            .map(|r| r.message.clone())
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// G-V1: modeled verdict criteria (§8.4.20.1)
// ---------------------------------------------------------------------------

/// §8.4.20.1: "Commonly, the verdict will only be pass if the objective … was
/// satisfied … However, this may not always be the desired condition for
/// passing, so the criteria for passing must be modeled explicitly."
///
/// A verification case may state its pass criterion via a `return verdict =
/// <expr>` result feature (VerificationCases.sysml:22 `return verdict :
/// VerdictKind :>> result`; the `PassIf` helper, :70-79). When present, that
/// criterion — NOT the worst-wins default over requirement checks — determines
/// the overall verdict. The requirement checks are retained for display/audit.
///
/// Two directions, each decoupling the requirement verdict (pinned via a
/// redefinition binding) from the criterion (driven by a subject attribute in
/// the eval context):
///   * requirement PASSES, criterion FAILS  → overall FAIL
///   * requirement FAILS,  criterion PASSES → overall PASS
#[test]
fn modeled_verdict_criterion_overrides_worst_wins() {
    let source = r#"
        package GV1 {
            requirement def MarginReq {
                attribute m;
                require constraint { m > 0 }
            }
            part def Widget { attribute margin; }

            // Requirement PASSES (m = 5 > 0) but the modeled criterion FAILS
            // (margin = 1.0, 1.0 > 2.0 is false).
            verification def CriterionFails {
                subject s : Widget;
                objective { verify requirement r : MarginReq { attribute m = 5; } }
                return verdict = margin > 2.0;
            }

            // Requirement FAILS (m = -5, -5 > 0 is false) but the modeled
            // criterion PASSES via PassIf (margin = 5.0, PassIf(5.0 > 2.0) = pass).
            verification def CriterionPassesViaPassIf {
                subject s : Widget;
                objective { verify requirement r : MarginReq { attribute m = -5; } }
                return verdict = PassIf(margin > 2.0);
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("gv1.sysml", source)]);
    elaborate(&mut result.graph);
    let runner = VerificationRunner::new();

    // Direction 1: passing requirement, failing criterion → FAIL.
    let fails = compile_verification_case("CriterionFails", &result.graph)
        .unwrap_or_else(|d| panic!("compile CriterionFails: {d:?}"));
    assert_eq!(
        fails.verdict_expression.as_deref(),
        Some("margin > 2.0"),
        "the modeled criterion must be lowered onto the IR"
    );
    let mut ctx = EvalContext::new();
    ctx.set("margin", Value::Float(1.0));
    let vr = runner.verify(&fails, &ctx);
    assert_eq!(
        vr.verdict,
        VerdictKind::Fail,
        "criterion says fail (1.0 > 2.0 == false) — must override the passing requirement check"
    );
    assert_eq!(
        vr.requirement_results.len(),
        1,
        "requirement checks are retained for display/audit"
    );
    assert_eq!(
        vr.requirement_results[0].verdict,
        VerdictKind::Pass,
        "the underlying requirement check itself PASSED (m = 5 > 0) — the override is at case level"
    );

    // Direction 2: failing requirement, passing criterion → PASS.
    let passes = compile_verification_case("CriterionPassesViaPassIf", &result.graph)
        .unwrap_or_else(|d| panic!("compile CriterionPassesViaPassIf: {d:?}"));
    assert_eq!(
        passes.verdict_expression.as_deref(),
        Some("PassIf(margin > 2.0)")
    );
    let mut ctx = EvalContext::new();
    ctx.set("margin", Value::Float(5.0));
    let vr = runner.verify(&passes, &ctx);
    assert_eq!(
        vr.verdict,
        VerdictKind::Pass,
        "PassIf(5.0 > 2.0) = pass — must override the failing requirement check"
    );
    assert_eq!(vr.requirement_results.len(), 1);
    assert_eq!(
        vr.requirement_results[0].verdict,
        VerdictKind::Fail,
        "the underlying requirement check itself FAILED (m = -5 > 0 == false)"
    );
}

/// Regression guard: a case with NO modeled criterion keeps the unchanged
/// default — worst-wins over the requirement checks (the spec's "commonly"
/// derivation, §8.4.20.1). This must not regress when the criterion path exists.
#[test]
fn absent_verdict_criterion_keeps_worst_wins_default() {
    let source = r#"
        package GV1Default {
            requirement def MarginReq {
                attribute m;
                require constraint { m > 0 }
            }
            part def Widget;
            verification def NoCriterion {
                subject s : Widget;
                objective { verify requirement r : MarginReq; }
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("gv1default.sysml", source)]);
    elaborate(&mut result.graph);

    let ir = compile_verification_case("NoCriterion", &result.graph)
        .unwrap_or_else(|d| panic!("compile NoCriterion: {d:?}"));
    assert_eq!(
        ir.verdict_expression, None,
        "no `return verdict = …` → no modeled criterion"
    );

    let runner = VerificationRunner::new();

    let mut ctx = EvalContext::new();
    ctx.set("m", Value::Float(5.0));
    assert_eq!(
        runner.verify(&ir, &ctx).verdict,
        VerdictKind::Pass,
        "default worst-wins: m = 5 > 0 passes"
    );

    ctx.set("m", Value::Float(-5.0));
    assert_eq!(
        runner.verify(&ir, &ctx).verdict,
        VerdictKind::Fail,
        "default worst-wins: m = -5 > 0 fails"
    );
}

/// A case's OWN owned attributes are in scope for its modeled criterion —
/// with an EMPTY caller context (KerML namespace semantics: owned features
/// are visible throughout the case body; the values ride the IR's `bindings`
/// so every consumer, including the service's batch context, gets them).
/// This mirrors the requirements-document idiom's interim-review shape:
/// the check FAILS but the case-owned-attribute-driven criterion PASSES.
#[test]
fn case_owned_attribute_is_in_scope_for_criterion() {
    let source = r#"
        package GV1Owned {
            requirement def EmcReq {
                attribute marginDb;
                require constraint { marginDb >= 3.0 }
            }
            part def Bench;
            verification def InterimReview {
                subject s : Bench;
                attribute declaredMarginDb = 2.5;
                objective { verify requirement r : EmcReq { attribute marginDb = 2.5; } }
                return verdict = PassIf(declaredMarginDb >= 2.0);
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("gv1owned.sysml", source)]);
    elaborate(&mut result.graph);

    let ir = compile_verification_case("InterimReview", &result.graph)
        .unwrap_or_else(|d| panic!("compile InterimReview: {d:?}"));
    assert!(
        ir.bindings
            .iter()
            .any(|(n, v)| n == "declaredMarginDb" && matches!(v, Value::Float(f) if *f == 2.5)),
        "the case-owned attribute value must ride the IR; got {:?}",
        ir.bindings
    );

    let runner = VerificationRunner::new();
    let vr = runner.verify(&ir, &EvalContext::new()); // deliberately empty
    assert_eq!(
        vr.verdict,
        VerdictKind::Pass,
        "criterion PassIf(2.5 >= 2.0) must PASS from the case's own scope \
         while the check fails (2.5 >= 3.0 is false); diagnostics: {:?}",
        vr.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert_eq!(
        vr.requirement_results.first().map(|r| r.verdict),
        Some(VerdictKind::Fail),
        "the check itself still fails and stays in the audit record"
    );
}

/// Fail-hard: a modeled criterion that cannot be evaluated (here, a free
/// variable that never binds) is an Error verdict with a diagnostic naming the
/// expression — never a silent fall-back to worst-wins.
#[test]
fn unevaluatable_verdict_criterion_is_error() {
    let source = r#"
        package GV1Bad {
            requirement def MarginReq {
                attribute m;
                require constraint { m > 0 }
            }
            part def Widget;
            verification def BadCriterion {
                subject s : Widget;
                objective { verify requirement r : MarginReq { attribute m = 5; } }
                return verdict = undefinedThing > 2.0;
            }
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("gv1bad.sysml", source)]);
    elaborate(&mut result.graph);

    let ir = compile_verification_case("BadCriterion", &result.graph)
        .unwrap_or_else(|d| panic!("compile BadCriterion: {d:?}"));
    assert_eq!(ir.verdict_expression.as_deref(), Some("undefinedThing > 2.0"));

    let runner = VerificationRunner::new();
    // Note: `margin`/`undefinedThing` intentionally unbound.
    let vr = runner.verify(&ir, &EvalContext::new());
    assert_eq!(
        vr.verdict,
        VerdictKind::Error,
        "an unevaluatable modeled criterion must fail hard, not silently fall back to worst-wins"
    );
    assert!(
        vr.diagnostics
            .iter()
            .any(|d| d.message.contains("undefinedThing > 2.0")),
        "the diagnostic must name the offending criterion; got {:?}",
        vr.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}
