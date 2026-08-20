//! Evaluation bridge for inline evaluation.
//!
//! Connects the expression evaluator and constraint checker from the
//! `sysml-run-*` crates to consumers, providing:
//! - Value inlay hints for calculations and attribute expressions
//! - Code lenses showing constraint pass/fail status
//! - Manual evaluation via the `sysml.evaluate` command

use std::collections::HashMap;
use sysml_core::{
    is_analysis_case_kind, is_verification_case_kind, Element, ElementKind, ModelGraph, Value,
};
use sysml_id::ElementId;
use sysml_runtime::actions::{compile_action, ActionRunner, ActionStepResult};
use sysml_runtime::cases::{
    compile_verification_case, value_to_json, RequirementCheck, RequirementResult, Verdict,
    VerdictContext, VerdictKind, VerificationRunner,
};
use sysml_runtime::constraints::EvalContext;
use sysml_runtime::expressions::{BinOp, ExprIR, compile_expression, ExpressionEvaluator};
use sysml_span::Span;

/// Result of evaluating a constraint element.
///
/// Carries both the legacy `satisfied` boolean (kept for backward compatibility
/// with existing call sites like `whatif.rs` and `aggregation.rs`) and the
/// universal `Verdict` shape locked in R1.3.
///
/// Round 1 populates `verdict.verdict` (VerdictKind) and `verdict.actual` with
/// the evaluated value. `expected`, `margin`, `sensitivity`, `evidence`, and
/// `metadata` are left empty; later rounds (2 — expected/margin extraction,
/// 4 — evidence wiring, 5 — sweep sensitivity) fill them in.
#[derive(Clone)]
pub struct EvalConstraintResult {
    pub element_id: ElementId,
    pub span: Option<Span>,
    /// Legacy boolean: true when the constraint expression evaluated to
    /// `Value::Bool(true)`. Preserved so existing aggregation/whatif consumers
    /// don't need to change shape.
    pub satisfied: bool,
    /// Universal verdict shape. Populated for every result — pass/fail/
    /// inconclusive/error — so downstream workflows can uniformly report
    /// `Verdict` without caring about the specific source.
    pub verdict: Verdict,
    /// Display string for code lenses.
    pub display: String,
    pub detail: String,
    /// Eval-result-loss collapse (C-soft-fallback-eval-result-loss): the
    /// `EvalError` display string when expression evaluation failed. `None`
    /// on the success path. Orthogonal to `verdict.actual` so consumers can
    /// distinguish "evaluator returned the string 'foo'" from "evaluator blew
    /// up with reason 'foo'".
    pub error: Option<String>,
}

/// Try to evaluate an expression for an element (for inlay hints).
///
/// Returns `Some(display_string)` if evaluation succeeds, `None` otherwise.
/// This is best-effort — failures are silently ignored.
pub fn try_evaluate_value(element: &Element, graph: &ModelGraph) -> Option<String> {
    let ir = compile_expression(element, graph).ok()?;
    let ctx = build_eval_context(element, graph);
    let evaluator = ExpressionEvaluator::new();
    let value = evaluator.eval(&ir, &ctx).ok()?;
    Some(format_value(&value))
}

/// Evaluate a specific element for the `sysml.evaluate` command.
///
/// Returns a JSON-friendly result string, or `None` if not evaluable.
pub fn evaluate_element(element: &Element, graph: &ModelGraph) -> Option<String> {
    let ir = compile_expression(element, graph).ok()?;
    let ctx = build_eval_context(element, graph);
    let evaluator = ExpressionEvaluator::new();
    match evaluator.eval(&ir, &ctx) {
        Ok(value) => Some(format_value(&value)),
        Err(e) => Some(format!("Error: {}", e)),
    }
}

/// Rich result for interactive expression evaluation.
///
/// This is the structured companion to the legacy `evaluate_element` string
/// helper. It reuses the same runtime evaluator but keeps value, verdict,
/// context, and diagnostics separate so UI workbenches can inspect them.
pub fn evaluate_expression_with_context(
    element: &Element,
    graph: &ModelGraph,
    ctx: &EvalContext,
) -> serde_json::Value {
    let source = get_expression_string(element, graph).unwrap_or_default();
    let ir = match compile_expression(element, graph) {
        Ok(ir) => ir,
        Err(diags) => {
            return serde_json::json!({
                "element_id": element.id.to_string(),
                "element_name": element.name,
                "source": source,
                "verdict": "error",
                "diagnostics": diags.iter().map(|d| d.message.clone()).collect::<Vec<_>>(),
                "context": serde_json::Map::new(),
            });
        }
    };

    let symbols = ir.free_variables();
    let mut context = serde_json::Map::new();
    for symbol in symbols.iter() {
        if let Some(value) = ctx.get(symbol) {
            context.insert(symbol.clone(), value_to_json(value));
        } else {
            context.insert(symbol.clone(), serde_json::Value::Null);
        }
    }

    let evaluator = ExpressionEvaluator::new();
    match evaluator.eval(&ir, ctx) {
        Ok(value) => {
            let verdict = match value {
                Value::Bool(true) => "pass",
                Value::Bool(false) => "fail",
                _ => "inconclusive",
            };
            serde_json::json!({
                "element_id": element.id.to_string(),
                "element_name": element.name,
                "source": source,
                "value": value_to_json(&value),
                "display": format_value(&value),
                "value_type": value_type_name(&value),
                "verdict": verdict,
                "symbols": symbols.into_iter().collect::<Vec<_>>(),
                "context": context,
                "diagnostics": [],
            })
        }
        Err(err) => serde_json::json!({
            "element_id": element.id.to_string(),
            "element_name": element.name,
            "source": source,
            "verdict": "error",
            "symbols": symbols.into_iter().collect::<Vec<_>>(),
            "context": context,
            "diagnostics": [err.to_string()],
        }),
    }
}

/// Evaluate all constraint elements in the graph (for code lenses).
///
/// This is the **static** (no-session) evaluation path: it produces a
/// verdict for every constraint expression but has no live runtime session
/// to point at, so emitted `Verdict.evidence` is `None` by design. Callers
/// that do have a session + tick (for example, `verify_with_simulation`)
/// should use [`evaluate_constraints_with_session`] instead so every failing
/// verdict carries an `EvidenceRef` pointing back at the simulation state
/// that produced it (R3.5 contract).
pub fn evaluate_constraints(graph: &ModelGraph) -> Vec<EvalConstraintResult> {
    evaluate_constraints_impl(graph, None)
}

/// Evaluate all constraint elements in the graph, attaching evidence that
/// points back at the supplied runtime session + tick.
///
/// Use this variant from any runtime-coupled verification path so the UI can
/// deep-link from a verdict card back to the simulation tick that produced
/// it. When there is no live session the static [`evaluate_constraints`]
/// entry point is correct — evidence legitimately stays `None` there.
pub fn evaluate_constraints_with_session(
    graph: &ModelGraph,
    verdict_ctx: &VerdictContext,
) -> Vec<EvalConstraintResult> {
    evaluate_constraints_impl(graph, Some(verdict_ctx))
}

// CONFORMS-REQUIRED (deferred): this path is definition/scope-static. It emits
// ONE verdict per constraint element, evaluated in an owner-scoped +
// ancestor-aware context (`build_eval_contexts_batch`). It does NOT multiply a
// constraint per occurrence of a multi-instance owner — the spec's
// per-occurrence BooleanEvaluation semantics (Constraints.sysml:23
// `constraintChecks :> booleanEvaluations`, Performances.kerml:94-102). The
// per-instance home already exists — `ModelCompiler::evaluate_constraints_per_instance`
// (sysml-runtime/compiler.rs), wired into `sysml.constraint.check`. Routing
// this consumer (and `sysml.evaluate.constraints`) onto that primitive is a
// separate, director-gated wave: it reshapes `EvalConstraintResult` (instance
// identity), ripples into the B8-enrichment loop + aggregation.rs (which assume
// 1:1 element→result cardinality), and requires a broad evaluate-baseline
// re-bless. The gap is not only cardinality: the context here is
// definition-static, so a constraint referencing `self` or an instance-specific
// value resolves against the wrong context even for a single-instance owner
// where per-occurrence routing would otherwise be byte-identical. (The
// reporting cardinality of an editor-facing surface — one code-lens per source
// declaration — is itself SPEC-SILENT; only the evaluation *semantics* are
// CONFORMS-REQUIRED.) Literal owner-scoped values ARE seen correctly today.
fn evaluate_constraints_impl(
    graph: &ModelGraph,
    verdict_ctx: Option<&VerdictContext>,
) -> Vec<EvalConstraintResult> {
    let mut results = Vec::new();

    // Step 1: Collect all constraint elements (exclude stdlib/library constraints)
    let constraint_elements: Vec<&Element> = graph
        .elements
        .values()
        .filter(|e| {
            matches!(
                e.kind,
                ElementKind::ConstraintUsage | ElementKind::AssertConstraintUsage
            ) && !graph.is_library_element(&e.id)
        })
        .collect();

    // Step 2: Build evaluation contexts in batch (once per owner)
    let contexts = build_eval_contexts_batch(&constraint_elements, graph);

    // Step 3: Evaluate each constraint using pre-built context
    let evaluator = ExpressionEvaluator::new();

    for element in constraint_elements {
        let ir = match compile_expression(element, graph) {
            Ok(ir) => ir,
            Err(diags) => {
                tracing::debug!(
                    element_id = %element.id,
                    name = ?element.name,
                    diagnostics = ?diags,
                    "skipping constraint evaluation: failed to compile expression"
                );
                continue;
            }
        };
        let expr_str = get_expression_string(element, graph)
            .unwrap_or_else(|| format!("{:?}", ir));

        // Use pre-built context, or fall back to empty if not found
        let ctx = contexts
            .get(&element.id)
            .map(|c| c.alias_live())
            .unwrap_or_else(|| build_eval_context(element, graph));

        match evaluator.eval(&ir, &ctx) {
            Ok(Value::Bool(satisfied)) => {
                let display = if satisfied { "PASS" } else { "FAIL" };
                let detail = format!("{} ({})", display, expr_str);
                let kind = if satisfied {
                    VerdictKind::Pass
                } else {
                    VerdictKind::Fail
                };
                // R3.5: when we have a live session context, every emitted
                // verdict carries an EvidenceRef pointing at this constraint
                // element at the current tick. Static callers (code-lens,
                // pre-flight) pass `None` and the verdict carries no
                // evidence — legitimately, because there's no session to
                // deep-link back to.
                let mut verdict =
                    Verdict::new(kind).with_actual(serde_json::Value::Bool(satisfied));
                if let Some(ctx) = verdict_ctx {
                    verdict = verdict.with_evidence_from_context(
                        ctx,
                        Some(element.id.to_string()),
                    );
                }
                results.push(EvalConstraintResult {
                    element_id: element.id.clone(),
                    span: element.spans.first().cloned(),
                    satisfied,
                    verdict,
                    display: display.to_owned(),
                    detail,
                    error: None,
                });
            }
            Ok(value) => {
                // Non-boolean result — still emit a verdict so consumers see
                // the actual value. We classify as Inconclusive since the
                // requirement/constraint semantics are ill-defined.
                let detail = format!("= {}", format_value(&value));
                let actual_json = value_to_json(&value);
                let mut verdict =
                    Verdict::new(VerdictKind::Inconclusive).with_actual(actual_json);
                if let Some(ctx) = verdict_ctx {
                    verdict = verdict.with_evidence_from_context(
                        ctx,
                        Some(element.id.to_string()),
                    );
                }
                results.push(EvalConstraintResult {
                    element_id: element.id.clone(),
                    span: element.spans.first().cloned(),
                    satisfied: false,
                    verdict,
                    display: "?".to_owned(),
                    detail,
                    error: None,
                });
            }
            Err(e) => {
                // Common case: the constraint depends on a variable that
                // only exists inside a running session (e.g. a simulation
                // state variable). In the static-eval path we can't
                // resolve it, so emit an Error verdict and surface the
                // evaluator's reason on the structured `error` field
                // (eval-result-loss collapse, C-soft-fallback). Prior to
                // P2 this was stuffed into `verdict.actual` as a String,
                // which made "ran fine, value is 'foo'" and "blew up with
                // reason 'foo'" indistinguishable downstream.
                let err_msg = e.to_string();
                let detail = format!("ERROR ({}): {}", expr_str, err_msg);
                let mut verdict = Verdict::new(VerdictKind::Error);
                if let Some(ctx) = verdict_ctx {
                    verdict = verdict.with_evidence_from_context(
                        ctx,
                        Some(element.id.to_string()),
                    );
                }
                results.push(EvalConstraintResult {
                    element_id: element.id.clone(),
                    span: element.spans.first().cloned(),
                    satisfied: false,
                    verdict,
                    display: "ERR".to_owned(),
                    detail,
                    error: Some(err_msg.clone()),
                });
                tracing::debug!(
                    element_id = %element.id,
                    name = ?element.name,
                    expression = %expr_str,
                    error = %err_msg,
                    "constraint evaluation failed — emitted Error verdict"
                );
            }
        }
    }

    results
}

/// Evaluate all calculation elements in the graph (for code lenses).
pub fn evaluate_calculations(graph: &ModelGraph) -> Vec<(ElementId, Option<Span>, String)> {
    let mut results = Vec::new();

    for element in graph.elements.values() {
        if element.kind != ElementKind::CalculationUsage {
            continue;
        }

        if let Some(display) = try_evaluate_value(element, graph) {
            results.push((
                element.id.clone(),
                element.spans.first().cloned(),
                format!("= {}", display),
            ));
        }
    }

    results
}

/// Result of evaluating a verification case element.
pub struct VerificationCaseResult {
    pub element_id: ElementId,
    pub span: Option<Span>,
    pub case_id: String,
    pub case_name: String,
    /// Fully qualified name off the ownership chain (`Pkg::Sub::Case`) —
    /// the model-structure grouping key for scale surfaces (History
    /// latest-status bands; the industry's "suite" is our containment).
    /// None when the case or an ancestor is unnamed.
    pub qualified_name: Option<String>,
    pub subject: Option<String>,
    /// DECLARED verification methods off the case's `@VerificationMethod`
    /// annotations (`kind : VerificationMethodKind[1..*]` — plural by
    /// spec), read from the graph via the one-home
    /// `sysml_core::metadata::verification_methods`. Model intent, layer
    /// (1) of the B10 taxonomy — never conflate with how a verdict was
    /// computed. Empty when the case declares none.
    pub methods: Vec<String>,
    pub verdict: VerdictKind,
    pub total_requirements: usize,
    pub passed_requirements: usize,
    pub display: String,
    pub requirements: Vec<serde_json::Value>,
    pub diagnostics: Vec<String>,
}

/// Evaluate all verification case elements in the graph (for code lenses).
///
/// For each `VerificationCaseDefinition` or `VerificationCaseUsage`, compiles
/// the case from the graph and runs the verification runner to produce a verdict.
///
/// CONFORMS-REQUIRED (deferred, separate research wave): like
/// [`evaluate_constraints_impl`] this uses the definition/scope-static
/// `build_eval_contexts_batch` context — one verdict per case element, with no
/// per-occurrence multiplication and no `subj` (system-under-test) instance
/// binding. Spec-correct per-instance verification (VerificationCases.sysml)
/// is NOT a copy of the constraint per-instance pattern — it requires threading
/// the verified subject per usage occurrence — so it is tracked as its own wave,
/// not folded into the constraint routing. Single-instance values are seen
/// correctly today.
pub fn evaluate_verification_cases(graph: &ModelGraph) -> Vec<VerificationCaseResult> {
    let mut results = Vec::new();

    // Step 1: Collect all verification case elements. Library elements are
    // excluded — the stdlib's abstract base features (`VerificationCase`,
    // `verificationCases`, `self`, …) are vocabulary, not user cases, and were
    // leaking into the rows as perpetual INCONCLUSIVE noise (same filter the
    // constraint/analysis collectors above already apply).
    let verification_elements: Vec<&Element> = graph
        .elements
        .values()
        .filter(|e| {
            is_verification_case_kind(e.kind.clone()) && !graph.is_library_element(&e.id)
        })
        .collect();

    // Step 2: Build evaluation contexts in batch (once per owner)
    let contexts = build_eval_contexts_batch(&verification_elements, graph);

    // Step 3: Evaluate each verification case using pre-built context
    let runner = VerificationRunner::new();

    for element in verification_elements {
        let case_name = match &element.name {
            Some(n) => n.clone(),
            None => {
                tracing::debug!(
                    element_id = %element.id,
                    "skipping verification case evaluation: unnamed case element"
                );
                continue;
            }
        };

        let case_ir = match compile_verification_case(&case_name, graph) {
            Ok(ir) => ir,
            Err(diags) => {
                tracing::debug!(
                    element_id = %element.id,
                    case_name = %case_name,
                    diagnostics = ?diags,
                    "skipping verification case evaluation: compile failed"
                );
                continue;
            }
        };

        // Use pre-built context, or fall back if not found
        let ctx = contexts
            .get(&element.id)
            .map(|c| c.alias_live())
            .unwrap_or_else(|| build_eval_context(element, graph));
        let result = runner.verify(&case_ir, &ctx);

        let total = result.requirement_results.len();
        let passed = result
            .requirement_results
            .iter()
            .filter(|r| r.verdict.is_pass())
            .count();

        let display = match result.verdict {
            VerdictKind::Pass => format!("PASS ({}/{})", passed, total),
            VerdictKind::Fail => format!("FAIL ({}/{} failed)", total - passed, total),
            VerdictKind::Inconclusive => "INCONCLUSIVE".to_owned(),
            VerdictKind::Error => "ERROR".to_owned(),
        };

        let evaluator = ExpressionEvaluator::new();
        let requirements = result
            .requirement_results
            .iter()
            .map(|requirement_result| {
                let requirement = find_requirement_check(&case_ir.requirements, &requirement_result.requirement_id);
                serialize_requirement_result(requirement_result, requirement, &ctx, &evaluator)
            })
            .collect();
        let diagnostics = result.diagnostics.iter().map(ToString::to_string).collect();

        results.push(VerificationCaseResult {
            element_id: element.id.clone(),
            span: element.spans.first().cloned(),
            case_id: case_ir.id.clone(),
            case_name,
            qualified_name: graph
                .build_qualified_name(&element.id)
                .map(|q| q.to_string()),
            subject: case_ir.subject.clone(),
            methods: sysml_core::metadata::verification_methods(graph, &element.id),
            verdict: result.verdict,
            total_requirements: total,
            passed_requirements: passed,
            display,
            requirements,
            diagnostics,
        });
    }

    results
}

fn find_requirement_check<'a>(requirements: &'a [RequirementCheck], id: &str) -> Option<&'a RequirementCheck> {
    for requirement in requirements {
        if requirement.id == id {
            return Some(requirement);
        }
        if let Some(found) = find_requirement_check(&requirement.subrequirements, id) {
            return Some(found);
        }
    }
    None
}

pub(crate) fn serialize_requirement_result_for_case(
    result: &RequirementResult,
    requirements: &[RequirementCheck],
    ctx: &EvalContext,
) -> serde_json::Value {
    let evaluator = ExpressionEvaluator::new();
    let requirement = find_requirement_check(requirements, &result.requirement_id);
    serialize_requirement_result(result, requirement, ctx, &evaluator)
}

fn serialize_requirement_result(
    result: &RequirementResult,
    requirement: Option<&RequirementCheck>,
    ctx: &EvalContext,
    evaluator: &ExpressionEvaluator,
) -> serde_json::Value {
    let measurements = requirement
        .map(|req| {
            req.constraints
                .iter()
                .map(|constraint| measure_constraint(constraint, ctx, evaluator))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let constraints = requirement
        .map(|req| {
            req.constraints
                .iter()
                .enumerate()
                .map(|(index, constraint)| {
                    let measurement = measurements.get(index);
                    serde_json::json!({
                        "index": index,
                        "constraint_id": req.constraint_element_ids.get(index).and_then(|id| id.clone()),
                        "element_id": req.constraint_element_ids.get(index).and_then(|id| id.clone()),
                        "expression_id": req.constraint_element_ids.get(index).and_then(|id| id.clone()),
                        "expression": expr_ir_to_string(constraint),
                        "symbols": constraint.free_variables().into_iter().collect::<Vec<_>>(),
                        "satisfied": result.constraints_met.get(index).copied(),
                        "actual": measurement.and_then(|m| m.actual.clone()),
                        "expected": measurement.and_then(|m| m.expected.clone()),
                        "margin": measurement.and_then(|m| m.margin),
                        "parts": measurement.map(|m| m.parts.clone()).unwrap_or_default(),
                        "error": measurement.and_then(|m| m.error.clone()),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let subrequirements = result
        .subrequirement_results
        .iter()
        .map(|sub_result| {
            let sub_requirement = requirement.and_then(|req| {
                find_requirement_check(&req.subrequirements, &sub_result.requirement_id)
            });
            serialize_requirement_result(sub_result, sub_requirement, ctx, evaluator)
        })
        .collect::<Vec<_>>();
    let first_measurement = measurements.first();
    let actual = first_measurement
        .and_then(|m| m.actual.clone())
        .unwrap_or_else(|| serde_json::Value::Bool(result.constraints_met.iter().all(|met| *met)));
    let expected = first_measurement
        .and_then(|m| m.expected.clone())
        .unwrap_or(serde_json::Value::Bool(true));
    let margin = first_measurement.and_then(|m| m.margin);
    let error = first_measurement.and_then(|m| m.error.clone());

    serde_json::json!({
        "requirement_id": result.requirement_id,
        "requirement_element_id": requirement.and_then(|req| req.source_element_id.clone()),
        "element_id": requirement.and_then(|req| req.source_element_id.clone()),
        "requirement_name": result.requirement_id,
        "requirement_text": requirement.and_then(|req| req.text.clone()),
        "verdict": format!("{}", result.verdict),
        "actual": actual,
        "expected": expected,
        "margin": margin,
        "error": error,
        "message": result.message,
        "assumptions_met": result.assumptions_met,
        "constraints_met": result.constraints_met,
        "constraints": constraints,
        "subrequirements": subrequirements,
    })
}

struct ConstraintMeasurement {
    actual: Option<serde_json::Value>,
    expected: Option<serde_json::Value>,
    margin: Option<f64>,
    parts: Vec<serde_json::Value>,
    error: Option<String>,
}

/// Eval-result-loss collapse helper: turn the typed `EvalError` into the
/// (value, error-display) pair that flows into `ConstraintMeasurement`.
///
/// Success keeps the value, failure surfaces the EvalError display string so
/// downstream consumers can distinguish "evaluator returned the string 'foo'"
/// from "evaluator blew up with reason 'foo'".
fn eval_into_parts(
    evaluator: &ExpressionEvaluator,
    expr: &ExprIR,
    ctx: &EvalContext,
) -> (Option<Value>, Option<String>) {
    match evaluator.eval(expr, ctx) {
        Ok(v) => (Some(v), None),
        Err(e) => (None, Some(e.to_string())),
    }
}

fn measure_constraint(
    constraint: &ExprIR,
    ctx: &EvalContext,
    evaluator: &ExpressionEvaluator,
) -> ConstraintMeasurement {
    if let Some(measurement) = measure_direct_comparison(constraint, ctx, evaluator) {
        return measurement;
    }

    let parts = collect_compound_measurements(constraint, ctx, evaluator);
    if !parts.is_empty() {
        let (actual, error) = eval_into_parts(evaluator, constraint, ctx);
        return ConstraintMeasurement {
            actual: actual.as_ref().map(value_to_json),
            expected: Some(serde_json::Value::Bool(true)),
            margin: aggregate_part_margin(constraint, &parts),
            parts,
            error,
        };
    }

    let (actual, error) = eval_into_parts(evaluator, constraint, ctx);
    ConstraintMeasurement {
        actual: actual.as_ref().map(value_to_json),
        expected: Some(serde_json::Value::Bool(true)),
        margin: None,
        parts: Vec::new(),
        error,
    }
}

fn measure_direct_comparison(
    expr: &ExprIR,
    ctx: &EvalContext,
    evaluator: &ExpressionEvaluator,
) -> Option<ConstraintMeasurement> {
    let ExprIR::BinaryOp { op, left, right } = expr else {
        return None;
    };
    if !is_comparison_op(*op) {
        return None;
    }

    let (left_value, left_err) = eval_into_parts(evaluator, left, ctx);
    let (right_value, right_err) = eval_into_parts(evaluator, right, ctx);
    let margin = match (
        left_value.as_ref().and_then(value_as_f64),
        right_value.as_ref().and_then(value_as_f64),
    ) {
        (Some(actual), Some(expected)) => Some(actual - expected),
        _ => None,
    };
    // Merge errors across the comparison: prefer the LEFT side (eval order),
    // fall back to the RIGHT if only the right failed.
    let error = left_err.or(right_err);
    Some(ConstraintMeasurement {
        actual: left_value.as_ref().map(value_to_json),
        expected: right_value.as_ref().map(value_to_json),
        margin,
        parts: Vec::new(),
        error,
    })
}

fn collect_compound_measurements(
    expr: &ExprIR,
    ctx: &EvalContext,
    evaluator: &ExpressionEvaluator,
) -> Vec<serde_json::Value> {
    let ExprIR::BinaryOp { op, left, right } = expr else {
        return Vec::new();
    };
    if !matches!(op, BinOp::And | BinOp::Or) {
        return Vec::new();
    }

    let mut parts = Vec::new();
    collect_compound_measurements_into(left, ctx, evaluator, &mut parts);
    collect_compound_measurements_into(right, ctx, evaluator, &mut parts);
    parts
}

fn collect_compound_measurements_into(
    expr: &ExprIR,
    ctx: &EvalContext,
    evaluator: &ExpressionEvaluator,
    parts: &mut Vec<serde_json::Value>,
) {
    if let Some(measurement) = measure_direct_comparison(expr, ctx, evaluator) {
        let (whole_value, whole_err) = eval_into_parts(evaluator, expr, ctx);
        // Prefer the per-comparison error (inner subexpression failure) when
        // measure_direct_comparison surfaced one; otherwise fall back to the
        // whole-expression failure.
        let error = measurement.error.clone().or(whole_err);
        parts.push(serde_json::json!({
            "expression": expr_ir_to_string(expr),
            "symbols": expr.free_variables().into_iter().collect::<Vec<_>>(),
            "satisfied": whole_value.as_ref().and_then(|value| value.as_bool()),
            "actual": measurement.actual,
            "expected": measurement.expected,
            "margin": measurement.margin,
            "error": error,
        }));
        return;
    }

    if let ExprIR::BinaryOp { op, left, right } = expr {
        if matches!(op, BinOp::And | BinOp::Or) {
            collect_compound_measurements_into(left, ctx, evaluator, parts);
            collect_compound_measurements_into(right, ctx, evaluator, parts);
        }
    }
}

fn aggregate_part_margin(expr: &ExprIR, parts: &[serde_json::Value]) -> Option<f64> {
    let margins = parts.iter().filter_map(|part| part.get("margin").and_then(|value| value.as_f64()));
    match expr {
        ExprIR::BinaryOp { op: BinOp::And, .. } => margins.reduce(f64::min),
        ExprIR::BinaryOp { op: BinOp::Or, .. } => margins.reduce(f64::max),
        _ => None,
    }
}

fn is_comparison_op(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Equal
            | BinOp::NotEqual
            | BinOp::ReferenceEqual
            | BinOp::ReferenceNotEqual
            | BinOp::LessThan
            | BinOp::LessEqual
            | BinOp::GreaterThan
            | BinOp::GreaterEqual
    )
}

fn value_as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Int(value) => Some(*value as f64),
        Value::Float(value) => Some(*value),
        _ => None,
    }
}

fn expr_ir_to_string(expr: &ExprIR) -> String {
    match expr {
        ExprIR::LiteralInt(value) => value.to_string(),
        ExprIR::LiteralReal(value) => value.to_string(),
        ExprIR::LiteralBool(value) => value.to_string(),
        ExprIR::LiteralString(value) => format!("\"{value}\""),
        ExprIR::LiteralQuantity { value, unit, .. } => format!("{value} [{unit}]"),
        ExprIR::LiteralNull => "null".to_owned(),
        ExprIR::FeatureRef(name) => name.clone(),
        ExprIR::FeatureChain(chain) => chain.join("."),
        // RSC-2.3 slot-bound forms display as their original spelling.
        ExprIR::SlotRef { name, .. } => name.clone(),
        ExprIR::SlotChainHead { names, .. } => names.join("."),
        ExprIR::BinaryOp { op, left, right } => {
            format!(
                "{} {} {}",
                expr_ir_child_to_string(left),
                bin_op_symbol(*op),
                expr_ir_child_to_string(right)
            )
        }
        ExprIR::UnaryOp { op, operand } => format!("{}{}", unary_op_symbol(*op), expr_ir_child_to_string(operand)),
        ExprIR::Conditional { condition, then_expr, else_expr } => format!(
            "if {} then {} else {}",
            expr_ir_to_string(condition),
            expr_ir_to_string(then_expr),
            expr_ir_to_string(else_expr)
        ),
        ExprIR::NullCoalescing { expr, default } => format!("{} ?? {}", expr_ir_child_to_string(expr), expr_ir_child_to_string(default)),
        ExprIR::Select { source, binding, predicate } => format!("{}->select {{ |{}| {} }}", expr_ir_child_to_string(source), binding, expr_ir_to_string(predicate)),
        ExprIR::Collect { source, binding, transform } => format!("{}->collect {{ |{}| {} }}", expr_ir_child_to_string(source), binding, expr_ir_to_string(transform)),
        ExprIR::Reject { source, binding, predicate } => format!("{}->reject {{ |{}| {} }}", expr_ir_child_to_string(source), binding, expr_ir_to_string(predicate)),
        ExprIR::ForAll { source, binding, predicate } => format!("{}->forAll {{ |{}| {} }}", expr_ir_child_to_string(source), binding, expr_ir_to_string(predicate)),
        ExprIR::Exists { source, binding, predicate } => format!("{}->exists {{ |{}| {} }}", expr_ir_child_to_string(source), binding, expr_ir_to_string(predicate)),
        ExprIR::Index { sequence, index } => format!("{}#({})", expr_ir_child_to_string(sequence), expr_ir_to_string(index)),
        ExprIR::FunctionCall { name, args } => format!("{}({})", name, args.iter().map(expr_ir_to_string).collect::<Vec<_>>().join(", ")),
        ExprIR::Sequence(items) => format!("({})", items.iter().map(expr_ir_to_string).collect::<Vec<_>>().join(", ")),
        ExprIR::Range { lower, upper } => format!("{}..{}", expr_ir_child_to_string(lower), expr_ir_child_to_string(upper)),
        ExprIR::MetaAccess { operand, is_double } => format!("{}{}", if *is_double { "@@" } else { "@" }, expr_ir_child_to_string(operand)),
        ExprIR::ConstructorCall { type_name, named_args } => format!(
            "new {}({})",
            type_name,
            named_args
                .iter()
                .map(|(name, value)| format!("{} = {}", name, expr_ir_to_string(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn expr_ir_child_to_string(expr: &ExprIR) -> String {
    match expr {
        ExprIR::BinaryOp { .. } | ExprIR::Conditional { .. } | ExprIR::NullCoalescing { .. } => {
            format!("({})", expr_ir_to_string(expr))
        }
        _ => expr_ir_to_string(expr),
    }
}

fn bin_op_symbol(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Subtract => "-",
        BinOp::Multiply => "*",
        BinOp::Divide => "/",
        BinOp::Remainder => "%",
        BinOp::Power => "^",
        BinOp::Equal => "==",
        BinOp::NotEqual => "!=",
        BinOp::ReferenceEqual => "===",
        BinOp::ReferenceNotEqual => "!==",
        BinOp::LessThan => "<",
        BinOp::LessEqual => "<=",
        BinOp::GreaterThan => ">",
        BinOp::GreaterEqual => ">=",
        BinOp::And => "and",
        BinOp::Or => "or",
        BinOp::Xor => "xor",
        BinOp::Implies => "implies",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::HasType => "hastype",
        BinOp::IsType => "istype",
        BinOp::As => "as",
        BinOp::Meta => "meta",
    }
}

fn unary_op_symbol(op: sysml_runtime::expressions::UnaryOp) -> &'static str {
    match op {
        sysml_runtime::expressions::UnaryOp::Negate => "-",
        sysml_runtime::expressions::UnaryOp::Plus => "+",
        sysml_runtime::expressions::UnaryOp::Not => "not ",
        sysml_runtime::expressions::UnaryOp::BitNot => "~",
    }
}

/// Result of evaluating an analysis case element.
pub struct AnalysisCaseResult {
    pub element_id: ElementId,
    pub span: Option<Span>,
    pub case_name: String,
    pub display: String,
    pub subject: Option<String>,
    pub objective: Option<String>,
    pub tool_name: Option<String>,
    pub tool_uri: Option<String>,
    pub parameters: Vec<serde_json::Value>,
    pub constraints: Vec<serde_json::Value>,
    pub result_expression: Option<String>,
    pub diagnostics: Vec<String>,
}

/// Evaluate all analysis case elements in the graph (for code lenses).
///
/// For each `AnalysisCaseDefinition` or `AnalysisCaseUsage`, compiles
/// the case from the graph and runs the solver to produce an output summary.
pub fn evaluate_analysis_cases(graph: &ModelGraph) -> Vec<AnalysisCaseResult> {
    let mut results = Vec::new();

    for element in graph.elements.values() {
        if !is_analysis_case_kind(element.kind.clone()) {
            continue;
        }

        let name = element
            .name
            .clone()
            .unwrap_or_else(|| "<unnamed>".to_owned());
        let span = element.spans.first().cloned();

        match sysml_runtime::cases::compile_analysis_case(&name, graph) {
            Ok(case_ir) => {
                // Build context from model defaults
                let mut ctx = EvalContext::new();
                for elem in graph.elements.values() {
                    if elem.kind == ElementKind::AttributeUsage {
                        if let Some(n) = &elem.name {
                            if let Some(val) =
                                elem.get_prop("default").or_else(|| elem.get_prop("value"))
                            {
                                std::sync::Arc::make_mut(&mut ctx.variables)
                                    .entry(n.clone())
                                    .or_insert_with(|| val.clone());
                            }
                        }
                    }
                }

                let registry = sysml_runtime::SolverRegistry::default();
                let display = match case_ir.execute(&registry, &ctx) {
                    Ok(r) => {
                        let count = r.outputs.len();
                        if r.converged {
                            format!("\u{2713} {} outputs", count)
                        } else {
                            format!("\u{26a0} {} outputs (not converged)", count)
                        }
                    }
                    Err(e) => format!("\u{2717} {}", e),
                };

                let parameters = case_ir
                    .parameters
                    .iter()
                    .map(|param| {
                        let direction = match param.direction {
                            sysml_runtime::solver_plugin::ParamDirection::In => "in",
                            sysml_runtime::solver_plugin::ParamDirection::Out => "out",
                            sysml_runtime::solver_plugin::ParamDirection::InOut => "inout",
                        };
                        serde_json::json!({
                            "name": param.sysml_name,
                            "tool_name": param.tool_name,
                            "value": param.value.as_ref().map(value_to_json),
                            "direction": direction,
                        })
                    })
                    .collect();
                let constraints = case_ir
                    .constraints
                    .iter()
                    .map(|constraint| {
                        serde_json::json!({
                            "expression": constraint.expr,
                            "description": constraint.description,
                            "owner_id": constraint.owner_id.as_ref().map(ToString::to_string),
                        })
                    })
                    .collect();

                results.push(AnalysisCaseResult {
                    element_id: element.id.clone(),
                    span,
                    case_name: name,
                    display,
                    subject: case_ir.subject.clone(),
                    objective: case_ir.objective.clone(),
                    tool_name: case_ir.tool_name.clone(),
                    tool_uri: case_ir.tool_uri.clone(),
                    parameters,
                    constraints,
                    result_expression: case_ir.result_expression.clone(),
                    diagnostics: Vec::new(),
                });
            }
            Err(diags) => {
                let diagnostics = diags.into_iter().map(|diag| diag.message).collect();
                results.push(AnalysisCaseResult {
                    element_id: element.id.clone(),
                    span,
                    case_name: name,
                    display: "? (compilation error)".to_owned(),
                    subject: None,
                    objective: None,
                    tool_name: None,
                    tool_uri: None,
                    parameters: Vec::new(),
                    constraints: Vec::new(),
                    result_expression: None,
                    diagnostics,
                });
            }
        }
    }

    results
}

/// Result of running an action to completion.
#[derive(Debug)]
pub struct ActionTraceResult {
    /// Trace of each step's outputs (for logging/display).
    pub steps: Vec<ActionStepResult>,
    /// Whether the action completed (reached final node).
    pub completed: bool,
    /// Total number of steps executed.
    pub total_steps: usize,
}

/// Run an action by name to completion and return a trace.
///
/// Compiles the action from the graph, builds an eval context, and steps
/// through the action runner until it completes (or hits the step limit).
pub fn run_action(
    action_name: &str,
    graph: &ModelGraph,
) -> Result<ActionTraceResult, String> {
    let ir = compile_action(action_name, graph).map_err(|diags| {
        diags
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("; ")
    })?;

    let mut runner = ActionRunner::new(ir);
    let ctx = EvalContext::new();
    let mut steps = Vec::new();
    let max_steps = 1000;

    loop {
        let result = runner.step(&ctx);
        let completed = result.completed;
        steps.push(result);
        if completed || steps.len() >= max_steps {
            break;
        }
    }

    let completed = runner.is_completed();
    let total_steps = steps.len();

    Ok(ActionTraceResult {
        steps,
        completed,
        total_steps,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Extract an expression display string from an element.
///
/// Prefers the canonical AST pretty-printer (structured expression subtree);
/// falls back to legacy `constraint` / `expr` string props only for
/// hand-crafted test graphs that have no AST children.
/// Public re-export of [`get_expression_string`] for callers in `lib.rs` that
/// need to enrich evaluate.* result rows with the source/pretty-printed
/// expression text (B8). Same semantics: AST pretty-printer first, then the
/// legacy `constraint` / `expr` string props for hand-crafted test graphs.
pub fn get_expression_string_public(element: &Element, graph: &ModelGraph) -> Option<String> {
    get_expression_string(element, graph)
}

fn get_expression_string(element: &Element, graph: &ModelGraph) -> Option<String> {
    // AST-first: structured expression subtree via pretty-printer
    if let Some(s) = sysml_core::expression_pretty::pretty_print_owner(element, graph) {
        return Some(s);
    }
    // Legacy fallback for test graphs without AST children
    if let Some(s) = element.get_prop("constraint").and_then(|v| v.as_str()) {
        return Some(s.to_owned());
    }
    if let Some(s) = element.get_prop("expr").and_then(|v| v.as_str()) {
        return Some(s.to_owned());
    }
    None
}

/// Build an evaluation context from the element's siblings.
///
/// Walks the owner's children to find sibling attributes with literal values.
/// Non-literal siblings are stored as `Value::Ref(ElementId)` for lazy
/// resolution during feature chain evaluation (no recursion, no depth limit).
pub fn build_eval_context(element: &Element, graph: &ModelGraph) -> EvalContext {
    build_eval_context_arc(element, &std::sync::Arc::new(graph.clone()))
}

/// Build eval context from an already-Arc'd graph (avoids clone).
///
/// RSC-3.1 (design doc D-3.0.6-B): the overlay is owner-scoped *and*
/// ancestor-aware — the constraint's immediate-owner attributes take
/// precedence, then each ancestor's attributes are layered underneath (a name
/// already provided by a nearer scope is never overwritten). This lets a
/// constraint reference an attribute declared in an enclosing part/definition,
/// matching the compile-time owner-scoped binder in the runtime orchestrator.
pub fn build_eval_context_arc(element: &Element, graph: &std::sync::Arc<ModelGraph>) -> EvalContext {
    let mut ctx = EvalContext::new();
    ctx.graph = Some(std::sync::Arc::clone(graph));

    if let Some(owner_id) = &element.owner {
        overlay_scope_siblings(&mut ctx, graph, owner_id, Some(&element.id), true);
        // Ancestor scopes (owner's owner, …) layered underneath — nearer
        // scopes win because we only set names not already present.
        for ancestor in sysml_core::query::ancestors(graph, owner_id) {
            overlay_scope_siblings(&mut ctx, graph, &ancestor.id, None, false);
        }
    }

    ctx
}

/// Build an owner-scoped evaluation context keyed directly on an *owner*
/// element id (rather than derived from a child element's `.owner`).
///
/// Mirrors [`build_eval_context_arc`] but lets a caller that holds only an
/// owner id — e.g. a precompiled
/// [`sysml_runtime::constraints::TypedConstraint`] carrying
/// `constraint.owner_id` — resolve the same owner-scoped + ancestor-aware
/// context. Owner siblings overlay first (value-less ones as lazy
/// `Value::Ref`), then ancestor literals underneath; nearer scope wins.
///
/// This is the one home for "scope a context to an owner": `whatif`'s
/// no-session path uses it so every constraint evaluates against the values
/// declared in *its* owner, not a flattened whole-graph map (which silently
/// collapsed same-named attributes across parts via HashMap last-writer-wins).
pub fn owner_scoped_context(
    graph: &std::sync::Arc<ModelGraph>,
    owner_id: &ElementId,
) -> EvalContext {
    let mut ctx = EvalContext::new();
    ctx.graph = Some(std::sync::Arc::clone(graph));
    overlay_scope_siblings(&mut ctx, graph, owner_id, None, true);
    for ancestor in sysml_core::query::ancestors(graph, owner_id) {
        overlay_scope_siblings(&mut ctx, graph, &ancestor.id, None, false);
    }
    ctx
}

/// Overlay the named attribute children of `scope_id` into `ctx`, skipping
/// `exclude` (the constraint itself) and any name already bound by a nearer
/// scope.
///
/// `ref_nonliterals` controls how value-less attributes are handled:
/// - **immediate owner** (`true`): the legacy behavior — literals stored
///   directly, non-literals stored as lazy [`Value::Ref`] (so feature chains
///   and expression-valued siblings resolve). This is the unchanged
///   owner-scope overlay.
/// - **ancestor scopes** (`false`, RSC-3.1 D-3.0.6-B): only attributes with a
///   concrete literal value are overlaid. A value-less ancestor attribute is
///   left out, so a constraint that references an unresolvable name keeps its
///   honest "undefined variable" verdict rather than gaining a value-less
///   `Ref` that merely refines the error string. The only verdicts an
///   ancestor scope can change are genuine ERROR→PASS flips on real values.
fn overlay_scope_siblings(
    ctx: &mut EvalContext,
    graph: &ModelGraph,
    scope_id: &ElementId,
    exclude: Option<&ElementId>,
    ref_nonliterals: bool,
) {
    for sibling in graph.children_of(scope_id) {
        if exclude == Some(&sibling.id) {
            continue;
        }
        if let Some(name) = &sibling.name {
            if ctx.get(name).is_some() {
                continue; // nearer scope already provided this name
            }
            if let Some(val) = extract_literal_value(sibling) {
                ctx.set(name.clone(), val);
            } else if ref_nonliterals {
                // Store as Ref for lazy resolution during eval_feature_chain.
                ctx.set(name.clone(), Value::Ref(sibling.id.clone()));
            }
            // else: ancestor-scope value-less attribute — left unbound so the
            // constraint's "undefined variable" verdict stays honest.
        }
    }
}

/// Build evaluation contexts for multiple elements sharing the same owner (batched).
///
/// This is more efficient than calling build_eval_context repeatedly for elements
/// with the same owner, as it walks the children only once per owner.
fn build_eval_contexts_batch(
    elements: &[&Element],
    graph: &ModelGraph,
) -> HashMap<ElementId, EvalContext> {
    let mut contexts = HashMap::new();

    // Group elements by owner_id
    let mut by_owner: HashMap<ElementId, Vec<&Element>> = HashMap::new();
    for element in elements {
        if let Some(owner_id) = &element.owner {
            by_owner.entry(owner_id.clone()).or_default().push(element);
        }
    }

    let arc_graph = std::sync::Arc::new(graph.clone());

    // Build one base context per owner
    for (owner_id, owned_elements) in by_owner {
        let mut base_ctx = EvalContext::new();

        base_ctx.graph = Some(std::sync::Arc::clone(&arc_graph));

        // Owner attributes first (Ref for non-literals, lazy resolution),
        // then ancestor attributes layered underneath (RSC-3.1 D-3.0.6-B:
        // owner-scoped + ancestor-aware overlay — nearer scope wins, so we
        // only set a name once). Mirrors the compile-time owner-scoped binder.
        overlay_scope_siblings(&mut base_ctx, graph, &owner_id, None, true);
        for ancestor in sysml_core::query::ancestors(graph, &owner_id) {
            overlay_scope_siblings(&mut base_ctx, graph, &ancestor.id, None, false);
        }

        // Clone the base context for each element, removing self
        for element in owned_elements {
            let mut ctx = base_ctx.alias_live();
            // Remove the element's own name from the context (can't reference self)
            if let Some(name) = &element.name {
                std::sync::Arc::make_mut(&mut ctx.variables).remove(name);
            }
            contexts.insert(element.id.clone(), ctx);
        }
    }

    contexts
}

/// Try to extract a literal value from an element's properties.
fn extract_literal_value(element: &Element) -> Option<Value> {
    // Check for explicit "value" property (set during elaboration)
    if let Some(val) = element.get_prop("value") {
        match val {
            Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::String(_) => {
                return Some(val.clone());
            }
            _ => {}
        }
    }
    // Check for "default" property
    if let Some(val) = element.get_prop("default") {
        match val {
            Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::String(_) => {
                return Some(val.clone());
            }
            _ => {}
        }
    }
    // Note: `unresolved_value` is no longer written by either parser
    // (removed in Phase 6D). Any expression data lives in the AST subtree,
    // which is out of scope for literal extraction.
    None
}

// Feature chain resolution is handled lazily by the expression evaluator
// using Value::Ref + ModelGraph lookup — no recursive map building needed.
// See `eval_feature_chain()` in `sysml-runtime/src/expressions/evaluator.rs`.

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Int(_) => "Int",
        Value::Float(_) => "Float",
        Value::Bool(_) => "Bool",
        Value::String(_) => "String",
        Value::Enum(_) => "Enum",
        Value::Null => "Null",
        Value::List(_) => "List",
        Value::Map(_) => "Map",
        Value::Ref(_) => "Ref",
        Value::Complex { .. } => "Complex",
        Value::Quantity { .. } => "Quantity",
    }
}

/// Format a Value for display in hints/lenses.
fn format_value(value: &Value) -> String {
    match value {
        Value::Int(i) => i.to_string(),
        Value::Float(f) => {
            // Avoid unnecessary precision for round numbers
            if *f == (*f as i64) as f64 {
                format!("{:.0}", f)
            } else {
                format!("{}", f)
            }
        }
        Value::Bool(b) => b.to_string(),
        Value::String(s) => format!("\"{}\"", s),
        Value::Null => "null".to_owned(),
        other => format!("{}", other),
    }
}

/// Evaluate constraints and return results grouped by owner element ID.
pub fn evaluate_constraints_grouped(
    graph: &ModelGraph,
) -> HashMap<ElementId, Vec<EvalConstraintResult>> {
    use std::collections::HashMap;

    let all_results = evaluate_constraints(graph);
    let mut grouped: HashMap<ElementId, Vec<EvalConstraintResult>> = HashMap::new();

    for result in all_results {
        if let Some(element) = graph.get_element(&result.element_id) {
            if let Some(owner_id) = &element.owner {
                grouped.entry(owner_id.clone()).or_default().push(result);
            }
        }
    }

    grouped
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::VisibilityKind;

    fn make_element(kind: ElementKind, name: &str) -> Element {
        Element::new(ElementId::new_v4(), kind).with_name(name)
    }

    #[test]
    fn try_evaluate_simple_arithmetic() {
        let graph = ModelGraph::new();
        let element = make_element(ElementKind::CalculationUsage, "power")
            .with_prop("expr", Value::String("2 + 3".into()));
        assert_eq!(try_evaluate_value(&element, &graph), Some("5".into()));
    }

    #[test]
    fn try_evaluate_no_expression_returns_none() {
        let graph = ModelGraph::new();
        let element = make_element(ElementKind::AttributeUsage, "speed");
        assert_eq!(try_evaluate_value(&element, &graph), None);
    }

    #[test]
    fn evaluate_constraint_passing() {
        let mut graph = ModelGraph::new();

        // Owner part
        let owner_id = ElementId::new_v4();
        let owner = Element::new(owner_id.clone(), ElementKind::PartUsage).with_name("vehicle");
        graph.add_element(owner);

        // Sibling: speed = 50
        let speed =
            make_element(ElementKind::AttributeUsage, "speed").with_prop("value", Value::Int(50));
        graph.add_owned_element(speed, owner_id.clone(), VisibilityKind::Public);

        // Constraint: speed < 100
        let constraint = make_element(ElementKind::ConstraintUsage, "speedLimit")
            .with_prop("constraint", Value::String("speed < 100".into()));
        let cid = graph.add_owned_element(constraint, owner_id.clone(), VisibilityKind::Public);

        let results = evaluate_constraints(&graph);
        let result = results.iter().find(|r| r.element_id == cid);
        assert!(result.is_some(), "constraint should produce a result");
        assert!(result.unwrap().satisfied, "speed=50 < 100 should pass");
    }

    #[test]
    fn evaluate_constraint_failing() {
        let mut graph = ModelGraph::new();

        let owner_id = ElementId::new_v4();
        let owner = Element::new(owner_id.clone(), ElementKind::PartUsage).with_name("vehicle");
        graph.add_element(owner);

        let speed =
            make_element(ElementKind::AttributeUsage, "speed").with_prop("value", Value::Int(150));
        graph.add_owned_element(speed, owner_id.clone(), VisibilityKind::Public);

        let constraint = make_element(ElementKind::ConstraintUsage, "speedLimit")
            .with_prop("constraint", Value::String("speed < 100".into()));
        let cid = graph.add_owned_element(constraint, owner_id.clone(), VisibilityKind::Public);

        let results = evaluate_constraints(&graph);
        let result = results.iter().find(|r| r.element_id == cid);
        assert!(result.is_some(), "constraint should produce a result");
        assert!(!result.unwrap().satisfied, "speed=150 < 100 should fail");

        // R3.5 contract: static path emits evidence = None legitimately.
        // (No runtime session exists for this call site.)
        assert!(
            result.unwrap().verdict.evidence.is_none(),
            "static evaluate_constraints must leave evidence = None"
        );
    }

    /// RSC-3.1 (D-3.0.6-B): a constraint owned by a nested part can reference
    /// an attribute declared in an ANCESTOR scope. Before ancestor-walking the
    /// reference was undefined (Error); now it resolves through the owner-scoped
    /// + ancestor-aware overlay.
    #[test]
    fn evaluate_constraint_resolves_ancestor_attribute() {
        let mut graph = ModelGraph::new();

        // Grandparent part carries `maxSpeed = 100`.
        let gp_id = ElementId::new_v4();
        graph.add_element(
            Element::new(gp_id.clone(), ElementKind::PartUsage).with_name("vehicle"),
        );
        let max_speed = make_element(ElementKind::AttributeUsage, "maxSpeed")
            .with_prop("value", Value::Int(100));
        graph.add_owned_element(max_speed, gp_id.clone(), VisibilityKind::Public);

        // Nested child part owns the constraint + its own `speed = 50`.
        let child_id = ElementId::new_v4();
        let child = Element::new(child_id.clone(), ElementKind::PartUsage).with_name("engine");
        graph.add_owned_element(child, gp_id.clone(), VisibilityKind::Public);
        let speed = make_element(ElementKind::AttributeUsage, "speed")
            .with_prop("value", Value::Int(50));
        graph.add_owned_element(speed, child_id.clone(), VisibilityKind::Public);

        // Constraint owned by the child references the ANCESTOR's maxSpeed.
        let constraint = make_element(ElementKind::ConstraintUsage, "withinMax")
            .with_prop("constraint", Value::String("speed < maxSpeed".into()));
        let cid = graph.add_owned_element(constraint, child_id, VisibilityKind::Public);

        let results = evaluate_constraints(&graph);
        let result = results
            .iter()
            .find(|r| r.element_id == cid)
            .expect("constraint should produce a result");
        assert!(
            result.error.is_none(),
            "ancestor attribute must resolve, not error: {:?}",
            result.error
        );
        assert!(
            result.satisfied,
            "speed(50) < maxSpeed(100) resolves via ancestor scope and passes"
        );
    }

    /// RSC-3.1: the immediate owner's attribute shadows a same-named ancestor
    /// attribute (nearer scope wins).
    #[test]
    fn evaluate_constraint_owner_shadows_ancestor() {
        let mut graph = ModelGraph::new();

        let gp_id = ElementId::new_v4();
        graph.add_element(
            Element::new(gp_id.clone(), ElementKind::PartUsage).with_name("outer"),
        );
        // Ancestor `limit = 10`.
        let outer_limit = make_element(ElementKind::AttributeUsage, "limit")
            .with_prop("value", Value::Int(10));
        graph.add_owned_element(outer_limit, gp_id.clone(), VisibilityKind::Public);

        let child_id = ElementId::new_v4();
        let child = Element::new(child_id.clone(), ElementKind::PartUsage).with_name("inner");
        graph.add_owned_element(child, gp_id.clone(), VisibilityKind::Public);
        // Owner `limit = 100` shadows the ancestor.
        let inner_limit = make_element(ElementKind::AttributeUsage, "limit")
            .with_prop("value", Value::Int(100));
        graph.add_owned_element(inner_limit, child_id.clone(), VisibilityKind::Public);

        let constraint = make_element(ElementKind::ConstraintUsage, "underLimit")
            .with_prop("constraint", Value::String("limit > 50".into()));
        let cid = graph.add_owned_element(constraint, child_id, VisibilityKind::Public);

        let results = evaluate_constraints(&graph);
        let result = results
            .iter()
            .find(|r| r.element_id == cid)
            .expect("constraint should produce a result");
        assert!(
            result.satisfied,
            "owner limit(100) > 50 must win over ancestor limit(10)"
        );
    }

    #[test]
    fn evaluate_constraints_with_session_populates_evidence() {
        // R3.5 runtime-coupled site: when a session context is provided,
        // every emitted Verdict.evidence points at (session_id, tick,
        // constraint_element_id).
        let mut graph = ModelGraph::new();

        let owner_id = ElementId::new_v4();
        let owner = Element::new(owner_id.clone(), ElementKind::PartUsage).with_name("vehicle");
        graph.add_element(owner);

        let speed =
            make_element(ElementKind::AttributeUsage, "speed").with_prop("value", Value::Int(150));
        graph.add_owned_element(speed, owner_id.clone(), VisibilityKind::Public);

        let constraint = make_element(ElementKind::ConstraintUsage, "speedLimit")
            .with_prop("constraint", Value::String("speed < 100".into()));
        let cid = graph.add_owned_element(constraint, owner_id.clone(), VisibilityKind::Public);

        let ctx = VerdictContext::new("sess-evidence-test", 99);
        let results = evaluate_constraints_with_session(&graph, &ctx);
        let result = results
            .iter()
            .find(|r| r.element_id == cid)
            .expect("constraint result present");
        assert!(!result.satisfied);

        let ev = result
            .verdict
            .evidence
            .as_ref()
            .expect("evidence populated for runtime-coupled failing verdict");
        assert_eq!(ev.session_id, "sess-evidence-test");
        assert_eq!(ev.tick, 99);
        assert_eq!(ev.element_id.as_deref(), Some(cid.to_string().as_str()));
    }

    #[test]
    fn evaluate_expression_with_overridden_context_returns_structured_result() {
        let mut graph = ModelGraph::new();

        let owner_id = ElementId::new_v4();
        let owner = Element::new(owner_id.clone(), ElementKind::PartUsage).with_name("vehicle");
        graph.add_element(owner);

        let speed =
            make_element(ElementKind::AttributeUsage, "speed").with_prop("value", Value::Int(50));
        graph.add_owned_element(speed, owner_id.clone(), VisibilityKind::Public);

        let constraint = make_element(ElementKind::ConstraintUsage, "speedLimit")
            .with_prop("constraint", Value::String("speed < 100".into()));
        let cid = graph.add_owned_element(constraint, owner_id.clone(), VisibilityKind::Public);
        let element = graph.get_element(&cid).expect("constraint exists");

        let mut ctx = build_eval_context(element, &graph);
        ctx.set("speed", Value::Int(150));
        let result = evaluate_expression_with_context(element, &graph, &ctx);

        assert_eq!(result["element_id"], cid.to_string());
        assert_eq!(result["verdict"], "fail");
        assert_eq!(result["value"], false);
        assert_eq!(result["context"]["speed"], 150);
    }

    #[test]
    fn build_context_extracts_sibling_values() {
        let mut graph = ModelGraph::new();

        let owner_id = ElementId::new_v4();
        let owner = Element::new(owner_id.clone(), ElementKind::PartDefinition).with_name("Part");
        graph.add_element(owner);

        let attr =
            make_element(ElementKind::AttributeUsage, "mass").with_prop("value", Value::Int(2400));
        graph.add_owned_element(attr, owner_id.clone(), VisibilityKind::Public);

        // The element we're building context for
        let target = make_element(ElementKind::ConstraintUsage, "check");
        let target_id = graph.add_owned_element(target, owner_id.clone(), VisibilityKind::Public);

        let target_element = graph.get_element(&target_id).unwrap();
        let ctx = build_eval_context(target_element, &graph);

        assert_eq!(ctx.get("mass"), Some(&Value::Int(2400)));
    }

    #[test]
    fn evaluate_element_returns_result() {
        let graph = ModelGraph::new();
        let element = make_element(ElementKind::CalculationUsage, "calc")
            .with_prop("expr", Value::String("10 * 5".into()));
        let result = evaluate_element(&element, &graph);
        assert_eq!(result, Some("50".into()));
    }

    #[test]
    fn evaluate_calculations_finds_calc_elements() {
        let mut graph = ModelGraph::new();
        let calc = make_element(ElementKind::CalculationUsage, "power")
            .with_prop("expr", Value::String("120 * 10".into()));
        let calc_id = calc.id.clone();
        graph.add_element(calc);

        let results = evaluate_calculations(&graph);
        assert!(
            results
                .iter()
                .any(|(id, _, display)| *id == calc_id && display.contains("1200")),
            "should find calc with result 1200"
        );
    }

    // -----------------------------------------------------------------------
    // Verification case evaluation tests
    // -----------------------------------------------------------------------

    /// Helper to build a graph with a verification case and owned requirements.
    fn build_verification_graph(
        case_name: &str,
        requirements: Vec<(&str, &str)>,
    ) -> (ModelGraph, ElementId) {
        let mut graph = ModelGraph::new();

        let vc_id = ElementId::new_v4();
        let vc = Element::new(vc_id.clone(), ElementKind::VerificationCaseDefinition)
            .with_name(case_name);
        graph.add_element(vc);

        for (req_name, constraint_expr) in requirements {
            let req = Element::new(ElementId::new_v4(), ElementKind::RequirementUsage)
                .with_name(req_name)
                .with_owner(vc_id.clone())
                .with_prop("constraint", Value::String(constraint_expr.into()));
            graph.add_element(req);
        }

        (graph, vc_id)
    }

    #[test]
    fn test_verification_case_pass() {
        // All requirements satisfied: speed(50) < 100
        let mut graph = ModelGraph::new();

        let owner_id = ElementId::new_v4();
        let owner = Element::new(owner_id.clone(), ElementKind::PartUsage).with_name("vehicle");
        graph.add_element(owner);

        let speed =
            make_element(ElementKind::AttributeUsage, "speed").with_prop("value", Value::Int(50));
        graph.add_owned_element(speed, owner_id.clone(), VisibilityKind::Public);

        let vc_id = ElementId::new_v4();
        let vc = Element::new(vc_id.clone(), ElementKind::VerificationCaseDefinition)
            .with_name("SpeedCheck");
        let vc_id = graph.add_owned_element(vc, owner_id.clone(), VisibilityKind::Public);

        let req = Element::new(ElementId::new_v4(), ElementKind::RequirementUsage)
            .with_name("speed-limit")
            .with_prop("constraint", Value::String("speed < 100".into()));
        graph.add_owned_element(req, vc_id.clone(), VisibilityKind::Public);

        let results = evaluate_verification_cases(&graph);
        let result = results.iter().find(|r| r.case_name == "SpeedCheck");
        assert!(result.is_some(), "should find verification case");
        let r = result.unwrap();
        assert!(
            matches!(r.verdict, VerdictKind::Pass),
            "speed=50 < 100 should pass, got {:?}",
            r.verdict
        );
        assert!(r.display.contains("PASS"));
    }

    #[test]
    fn test_verification_case_fail() {
        // Requirement not satisfied: true literal constraint that evaluates false
        let (graph, _vc_id) = build_verification_graph("FailCheck", vec![("req1", "1 > 2")]);

        let results = evaluate_verification_cases(&graph);
        let result = results.iter().find(|r| r.case_name == "FailCheck");
        assert!(result.is_some(), "should find verification case");
        let r = result.unwrap();
        assert!(
            matches!(r.verdict, VerdictKind::Fail),
            "1 > 2 should fail, got {:?}",
            r.verdict
        );
        assert!(r.display.contains("FAIL"));
    }

    #[test]
    fn test_verification_case_includes_requirement_and_constraint_source_ids() {
        let (graph, _vc_id) = build_verification_graph("TraceableCheck", vec![("req1", "1 < 2")]);

        let requirement_id = graph
            .elements
            .values()
            .find(|element| element.name.as_deref() == Some("req1"))
            .expect("requirement element exists")
            .id
            .to_string();

        let results = evaluate_verification_cases(&graph);
        let result = results.iter().find(|r| r.case_name == "TraceableCheck").expect("case result");
        let requirement = result.requirements.first().expect("requirement payload");

        assert_eq!(
            requirement.get("requirement_element_id").and_then(|v| v.as_str()),
            Some(requirement_id.as_str())
        );
        assert_eq!(
            requirement.get("element_id").and_then(|v| v.as_str()),
            Some(requirement_id.as_str())
        );
        let constraint = requirement
            .get("constraints")
            .and_then(|v| v.as_array())
            .and_then(|items| items.first())
            .expect("constraint payload");
        assert_eq!(
            constraint.get("constraint_id").and_then(|v| v.as_str()),
            Some(requirement_id.as_str())
        );
        assert_eq!(
            constraint.get("expression_id").and_then(|v| v.as_str()),
            Some(requirement_id.as_str())
        );
    }

    #[test]
    fn test_verification_case_compound_constraint_measurements() {
        let (graph, _vc_id) = build_verification_graph(
            "CompoundMeasurementCheck",
            vec![("req1", "1 < 2 and 5 < 4")],
        );

        let results = evaluate_verification_cases(&graph);
        let result = results
            .iter()
            .find(|r| r.case_name == "CompoundMeasurementCheck")
            .expect("case result");
        let requirement = result.requirements.first().expect("requirement payload");
        let constraint = requirement
            .get("constraints")
            .and_then(|v| v.as_array())
            .and_then(|items| items.first())
            .expect("constraint payload");

        assert_eq!(constraint.get("actual").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(constraint.get("expected").and_then(|v| v.as_bool()), Some(true));
        let parts = constraint
            .get("parts")
            .and_then(|v| v.as_array())
            .expect("compound constraint parts");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].get("expression").and_then(|v| v.as_str()), Some("1 < 2"));
        assert_eq!(parts[0].get("actual").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(parts[0].get("expected").and_then(|v| v.as_i64()), Some(2));
        assert_eq!(parts[0].get("satisfied").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(parts[1].get("expression").and_then(|v| v.as_str()), Some("5 < 4"));
        assert_eq!(parts[1].get("actual").and_then(|v| v.as_i64()), Some(5));
        assert_eq!(parts[1].get("expected").and_then(|v| v.as_i64()), Some(4));
        assert_eq!(parts[1].get("satisfied").and_then(|v| v.as_bool()), Some(false));
    }

    #[test]
    fn test_verification_case_multi_requirement() {
        // Mixed: one passes, one fails
        let (graph, _vc_id) = build_verification_graph(
            "MixedCheck",
            vec![("req-pass", "1 < 2"), ("req-fail", "3 < 1")],
        );

        let results = evaluate_verification_cases(&graph);
        let result = results.iter().find(|r| r.case_name == "MixedCheck");
        assert!(result.is_some(), "should find verification case");
        let r = result.unwrap();
        assert!(
            matches!(r.verdict, VerdictKind::Fail),
            "mixed should be FAIL, got {:?}",
            r.verdict
        );
        assert_eq!(r.total_requirements, 2);
        assert_eq!(r.passed_requirements, 1);
    }

    #[test]
    fn test_verification_case_inconclusive() {
        // Constraint references undefined variable -> inconclusive
        let (graph, _vc_id) =
            build_verification_graph("InconclusiveCheck", vec![("req1", "unknown_var < 100")]);

        let results = evaluate_verification_cases(&graph);
        let result = results.iter().find(|r| r.case_name == "InconclusiveCheck");
        assert!(result.is_some(), "should find verification case");
        let r = result.unwrap();
        assert!(
            matches!(r.verdict, VerdictKind::Inconclusive),
            "undefined var should be inconclusive, got {:?}",
            r.verdict
        );
        assert!(r.display.contains("INCONCLUSIVE"));
    }

    // -----------------------------------------------------------------------
    // Action execution trace tests
    // -----------------------------------------------------------------------

    /// Build a simple sequential action graph for testing.
    fn build_sequential_action_graph(action_name: &str) -> ModelGraph {
        let mut graph = ModelGraph::new();

        let action =
            Element::new(ElementId::new_v4(), ElementKind::ActionDefinition).with_name(action_name);
        let action_id = graph.add_element(action);

        // Add a child action step (assignment-like)
        let step1 = Element::new(ElementId::new_v4(), ElementKind::ActionUsage)
            .with_name("step1")
            .with_owner(action_id.clone());
        graph.add_element(step1);

        graph
    }

    #[test]
    fn test_action_run_sequential() {
        let graph = build_sequential_action_graph("ProcessData");
        let result = run_action("ProcessData", &graph);
        assert!(result.is_ok(), "sequential action should compile and run");
        let trace = result.unwrap();
        assert!(trace.completed, "action should complete");
        assert!(trace.total_steps > 0, "should have at least one step");
    }

    #[test]
    fn test_action_run_not_found() {
        let graph = ModelGraph::new();
        let result = run_action("NonExistent", &graph);
        assert!(result.is_err(), "should error for missing action");
        let err = result.unwrap_err();
        assert!(
            err.contains("not found"),
            "error should mention not found: {}",
            err
        );
    }

    #[test]
    fn test_action_run_with_assignment() {
        // Build a graph with an action that has an assign node
        let mut graph = ModelGraph::new();

        let action = Element::new(ElementId::new_v4(), ElementKind::ActionDefinition)
            .with_name("ComputeAction");
        let action_id = graph.add_element(action);

        // Add an AssignmentActionUsage child
        let assign = Element::new(ElementId::new_v4(), ElementKind::AssignmentActionUsage)
            .with_name("setX")
            .with_owner(action_id.clone())
            .with_prop("target", Value::String("x".into()))
            .with_prop("value_expr", Value::String("42".into()));
        graph.add_element(assign);

        let result = run_action("ComputeAction", &graph);
        assert!(result.is_ok(), "action with assignment should compile");
        let trace = result.unwrap();
        assert!(trace.completed, "action should complete");
    }
}
