//! RSC-CALC — Calculations spec-conformance harness.
//!
//! Fan-out area of the spec-derived semantic-conformance sweep (see
//! `spec-obligations/README.md`). Covers the **calculations** area. Same
//! convention: one obligation per test, a `// VERDICT:` marker, an `// OBL:`
//! line tying it to `spec-obligations/calculations.md`, and a self-scanning
//! `rsccalc_matrix_summary`.
//!
//! Spec sources (verified; cited in the tracker):
//! - SysML §7.19 "Calculations" (`SysML-spec-r2025-04_REF.html`)
//! - KerML §7.4.8 "Functions / result expression"
//! - `sysml.library/Systems Library/Calculations.sysml`
//!
//! Harness rules: pure runtime. A `CalculationDefinition` is built as a
//! ModelGraph, compiled with `CalculationRegistry::compile_all_from_graph`, and
//! evaluated with `evaluate_calculation` — the stable compile→evaluate path
//! (mirrors `calculations.rs`'s own unit tests).

use sysml_core::{Element, ElementId, ElementKind, ModelGraph, Value};
use sysml_runtime::calculations::{evaluate_calculation, CalculationRegistry};
use sysml_runtime::constraints::EvalContext;

/// Build a `calc def` with the given result expression and `in` parameters
/// (each optionally carrying a default).
fn build_calc(name: &str, expr: &str, params: &[(&str, Option<Value>)]) -> ModelGraph {
    let mut graph = ModelGraph::new();
    let calc_id = ElementId::new_v4();
    let mut calc = Element::new(calc_id.clone(), ElementKind::CalculationDefinition);
    calc.name = Some(name.to_string());
    calc.set_prop("result", Value::String(expr.to_string()));
    graph.add_element(calc);
    for (pname, default) in params {
        let mut p = Element::new(ElementId::new_v4(), ElementKind::AttributeUsage);
        p.name = Some((*pname).to_string());
        p.set_prop("direction", Value::String("in".to_string()));
        if let Some(d) = default {
            p.set_prop("default", d.clone());
        }
        p.owner = Some(calc_id.clone());
        graph.add_element(p);
    }
    graph
}

fn eval(name: &str, expr: &str, params: &[(&str, Option<Value>)], args: &[(&str, Value)]) -> Value {
    let graph = build_calc(name, expr, params);
    let (reg, diags) = CalculationRegistry::compile_all_from_graph(&graph);
    assert!(diags.is_empty(), "compile diags: {diags:?}");
    let calc = reg.get(name).expect("calc compiled");
    let args: Vec<(String, Value)> = args.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    evaluate_calculation(calc, &args, &EvalContext::new()).expect("evaluation")
}

// ===========================================================================
// OBL: calc-result-is-expression-value
// A calculation returns, in its result parameter, the value of evaluating its
// result expression. — SysML §7.19.1; KerML §7.4.8.3 (result expression bound
// to result parameter).
// ===========================================================================

#[test]
fn calc_result_is_the_evaluated_expression() {
    // OBL: calc-result-is-expression-value
    // VERDICT: CONFORMS
    let r = eval("Add", "a + b", &[("a", None), ("b", None)],
                 &[("a", Value::Float(3.0)), ("b", Value::Float(4.0))]);
    assert_eq!(r, Value::Float(7.0));
}

// ===========================================================================
// OBL: invocation-binds-arguments-to-input-params
// Input parameters are bound to the corresponding argument values. — KerML §7.4.9.1.
// ===========================================================================

#[test]
fn calc_inputs_bind_to_arguments() {
    // OBL: invocation-binds-arguments-to-input-params
    // VERDICT: CONFORMS
    // Different arguments ⇒ different results: the params genuinely bind.
    let lo = eval("Scale", "x * 2", &[("x", None)], &[("x", Value::Float(5.0))]);
    let hi = eval("Scale", "x * 2", &[("x", None)], &[("x", Value::Float(10.0))]);
    assert_eq!(lo, Value::Float(10.0));
    assert_eq!(hi, Value::Float(20.0));
}

// ===========================================================================
// OBL: calc-default-param-applied-when-arg-absent
// A declared parameter default is used when no argument is supplied. (Library
// FeatureValue default semantics; KerML feature default.)
// ===========================================================================

#[test]
fn calc_uses_default_when_argument_omitted() {
    // OBL: calc-default-param-applied-when-arg-absent
    // VERDICT: CONFORMS
    let r = eval("Scale", "x * factor",
                 &[("x", None), ("factor", Some(Value::Float(2.0)))],
                 &[("x", Value::Float(5.0))]); // factor omitted ⇒ default 2.0
    assert_eq!(r, Value::Float(10.0));
}

// ===========================================================================
// OBL: calc-always-has-result-parameter
// A calculation always has a result parameter (inherited if not owned); the
// engine always produces a result value. — SysML §7.19.2.
// ===========================================================================

#[test]
fn calc_always_produces_a_result_value() {
    // OBL: calc-always-has-result-parameter
    // VERDICT: CONFORMS
    let r = eval("Const", "42", &[], &[]);
    assert_eq!(r, Value::Int(42));
}

// ===========================================================================
// Matrix summary.
// ===========================================================================

#[test]
fn rsccalc_matrix_summary() {
    let src = include_str!("calculation_spec_conformance.rs");
    let verdicts: Vec<&str> = src
        .lines()
        .map(|l| l.trim())
        .filter(|l| l.starts_with("// VERDICT: "))
        .collect();
    let conforms = verdicts.iter().filter(|l| l.starts_with("// VERDICT: CONFORMS")).count();
    let diverges = verdicts.iter().filter(|l| l.starts_with("// VERDICT: DIVERGES")).count();
    let unimpl = verdicts.iter().filter(|l| l.starts_with("// VERDICT: UNIMPLEMENTED")).count();
    println!(
        "RSC-CALC calculations matrix: {} gated obligations — {conforms} CONFORMS, {diverges} DIVERGES, {unimpl} UNIMPLEMENTED",
        verdicts.len()
    );
    assert!(verdicts.len() >= 4);
}
