//! End-to-end pipeline tests for constraint evaluation.
//!
//! Tests the full path: build ModelGraph -> elaborate -> extract -> precompile -> evaluate.

use sysml_core::elaborate::elaborate;
use sysml_core::{Element, ElementKind, ModelGraph, Value};
use sysml_runtime::constraints::{
    compile_satisfy_requirement, evaluate_requirement, extract_and_precompile, EvalContext,
};

// ---------------------------------------------------------------------------
// C2: Constraint evaluation pipeline
// ---------------------------------------------------------------------------

/// Full pipeline: build constraint elements -> elaborate -> extract -> evaluate.
#[test]
fn constraint_elaborate_extract_evaluate() {
    let mut graph = ModelGraph::new();

    // Create constraint usages with expr prop (as parser pipeline produces).
    // Note: parsers no longer write `unresolved_value` (Phase 6D); use `constraint`
    // string prop directly for test graphs without AST children.
    let c1 = Element::new_with_kind(ElementKind::ConstraintUsage)
        .with_name("SpeedLimit")
        .with_prop("constraint", "speed < 100");
    graph.add_element(c1);

    let c2 = Element::new_with_kind(ElementKind::ConstraintUsage)
        .with_name("TempRange")
        .with_prop("constraint", "temp > 0");
    graph.add_element(c2);

    // Elaborate: constraints already have `constraint` prop, so no derivation needed
    let _report = elaborate(&mut graph);

    // Extract and pre-compile
    let precompiled = extract_and_precompile(&graph);
    assert_eq!(
        precompiled.compiled_count(),
        2, // 2 constraints, each compiled via compile_expression (string prop fallback)
        "Should compile all constraints"
    );
    assert_eq!(precompiled.failed_count(), 0, "No compilation failures");

    // Evaluate
    let mut ctx = EvalContext::new();
    ctx.set("speed", Value::Float(80.0));
    ctx.set("temp", Value::Float(25.0));

    let results = precompiled.evaluate_all(&ctx);
    assert!(
        results.iter().all(|r| r.satisfied),
        "All constraints should be satisfied"
    );

    // Now violate one
    ctx.set("speed", Value::Float(150.0));
    let results = precompiled.evaluate_all(&ctx);
    let failed: Vec<_> = results.iter().filter(|r| !r.satisfied).collect();
    assert!(
        !failed.is_empty(),
        "Should have at least one failed constraint"
    );
}

/// Pipeline with requirement assume/require roles.
#[test]
fn requirement_with_roles_pipeline() {
    let mut graph = ModelGraph::new();

    // Create a SatisfyRequirementUsage
    let req = Element::new_with_kind(ElementKind::SatisfyRequirementUsage).with_name("SafeSpeed");
    let req_id = graph.add_element(req);

    // Assumption child with constraintKind (as parser produces)
    let assume = Element::new_with_kind(ElementKind::ConstraintUsage)
        .with_name("EngineRunning")
        .with_owner(req_id.clone())
        .with_prop("constraintKind", "assumption")
        .with_prop("unresolved_value", "engine_on == 1");
    graph.add_element(assume);

    // Requirement child
    let require = Element::new_with_kind(ElementKind::ConstraintUsage)
        .with_name("SpeedLimit")
        .with_owner(req_id.clone())
        .with_prop("constraintKind", "requirement")
        .with_prop("unresolved_value", "speed < 200");
    graph.add_element(require);

    // Elaborate: should tag roles and copy constraint values
    let report = elaborate(&mut graph);
    assert!(
        report.constraints_derived > 0,
        "Should derive constraint properties"
    );

    // Compile requirement from graph
    let req_elem = graph.get_element(&req_id).unwrap();
    let compiled = compile_satisfy_requirement(req_elem, &graph).unwrap();

    assert_eq!(compiled.name, "SafeSpeed");
    assert_eq!(compiled.assumptions.len(), 1);
    assert_eq!(compiled.constraints.len(), 1);

    // Evaluate: assumption met, constraint met
    let mut ctx = EvalContext::new();
    ctx.set("engine_on", Value::Int(1));
    ctx.set("speed", Value::Float(100.0));
    let result = evaluate_requirement(&compiled, &ctx);
    assert!(result.satisfied);

    // Evaluate: assumption not met -> vacuously satisfied
    ctx.set("engine_on", Value::Int(0));
    ctx.set("speed", Value::Float(999.0));
    let result = evaluate_requirement(&compiled, &ctx);
    assert!(result.satisfied, "Should be vacuously satisfied");
}

/// Pipeline with negated assert constraint.
#[test]
fn negated_assert_constraint_pipeline() {
    let mut graph = ModelGraph::new();

    // AssertConstraintUsage with isNegated="true" (string, as parser might set)
    let c = Element::new_with_kind(ElementKind::AssertConstraintUsage)
        .with_name("NotOverheating")
        .with_prop("isNegated", "true")
        .with_prop("unresolved_value", "temp > 500");
    let c_id = graph.add_element(c);

    let report = elaborate(&mut graph);
    assert!(report.constraints_derived > 0);

    // Verify isNegated was normalized to bool
    let elem = graph.get_element(&c_id).unwrap();
    assert_eq!(
        elem.get_prop("isNegated").and_then(|v| v.as_bool()),
        Some(true),
        "isNegated should be normalized to bool"
    );
}
