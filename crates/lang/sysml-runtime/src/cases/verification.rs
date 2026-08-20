//! Verification-case runner and its result type.

use crate::expressions::{compile_simple_expression, EvalContext, EvaluationError, ExprIR, ExpressionEvaluator};
use sysml_core::Value;
use sysml_span::Diagnostic;

#[allow(unused_imports)]
use super::*;

// ---------------------------------------------------------------------------
// Verification case runner
// ---------------------------------------------------------------------------

/// Result of executing a verification case.
#[derive(Debug, Clone)]
pub struct VerificationResult {
    /// Overall verdict.
    pub verdict: VerdictKind,
    /// Individual requirement results.
    pub requirement_results: Vec<RequirementResult>,
    /// Diagnostics (errors/warnings).
    pub diagnostics: Vec<Diagnostic>,
}

/// Executes a verification case against a set of requirements.
///
/// # Example
///
/// ```
/// use sysml_runtime::cases::{VerificationRunner, VerificationCaseIR, RequirementCheck, VerdictKind};
/// use sysml_runtime::expressions::{EvalContext, ExprIR, BinOp};
/// use sysml_core::Value;
///
/// let case = VerificationCaseIR {
///     id: "vc1".into(),
///     name: "Speed Check".into(),
///     subject: Some("vehicle".into()),
///     setup_actions: vec![],
///     requirements: vec![
///         RequirementCheck {
///             id: "req1".into(),
///             source_element_id: None,
///             text: Some("Speed must be under limit".into()),
///             assumptions: vec![],
///             constraints: vec![
///                 ExprIR::BinaryOp {
///                     op: BinOp::LessThan,
///                     left: Box::new(ExprIR::FeatureRef("speed".into())),
///                     right: Box::new(ExprIR::FeatureRef("limit".into())),
///                 },
///             ],
///             constraint_element_ids: vec![None],
///             compile_errors: vec![],
///             subrequirements: vec![],
///             bindings: vec![],
///             binding_specs: vec![],
///         },
///     ],
///     sub_cases: vec![],
///     verdict_expression: None,
///     bindings: vec![],
/// };
///
/// let mut ctx = EvalContext::new();
/// ctx.set("speed", Value::Float(85.0));
/// ctx.set("limit", Value::Float(100.0));
///
/// let runner = VerificationRunner::new();
/// let result = runner.verify(&case, &ctx);
/// assert_eq!(result.verdict, VerdictKind::Pass);
/// ```
pub struct VerificationRunner {
    evaluator: ExpressionEvaluator,
    /// Optional runtime session context used to stamp every emitted
    /// [`Verdict`]'s `evidence` with `{ session_id, tick, element_id }`.
    ///
    /// When `None`, `verify(..)` still runs — but any downstream conversion
    /// to [`Verdict`] will have `evidence = None`, legitimately, because the
    /// caller is running in a static (no-session) context such as code-lens
    /// or pre-flight checking.
    verdict_ctx: Option<VerdictContext>,
}

impl VerificationRunner {
    /// Create a new verification runner with no attached session context.
    ///
    /// Verdicts produced via this runner carry `evidence: None` — appropriate
    /// for static / pre-flight use. For live runtime verification, prefer
    /// [`VerificationRunner::with_context`] so every emitted verdict deep-
    /// links back to the simulation state that produced it.
    pub fn new() -> Self {
        Self {
            evaluator: ExpressionEvaluator::new(),
            verdict_ctx: None,
        }
    }

    /// Create a verification runner that stamps every lifted `Verdict` with
    /// evidence derived from the given runtime session context.
    pub fn with_context(ctx: VerdictContext) -> Self {
        Self {
            evaluator: ExpressionEvaluator::new(),
            verdict_ctx: Some(ctx),
        }
    }

    /// Return the session context, if any.
    pub fn verdict_ctx(&self) -> Option<&VerdictContext> {
        self.verdict_ctx.as_ref()
    }

    /// Lift a [`RequirementResult`] into a [`Verdict`], stamping evidence
    /// from this runner's session context when it has one.
    ///
    /// Requires the `serde` feature (the `Verdict` body only carries data
    /// there). On non-serde builds this method still compiles and returns
    /// the stub variant without evidence serialization — mirroring the
    /// existing `Verdict::new` split.
    #[cfg(feature = "serde")]
    pub fn lift_requirement_result(&self, result: &RequirementResult) -> Verdict {
        match &self.verdict_ctx {
            Some(ctx) => Verdict::from_requirement_result_with_evidence(result, ctx),
            None => result.into(),
        }
    }

    /// Lift a [`VerificationResult`] aggregate into a [`Verdict`], stamping
    /// evidence from this runner's session context when it has one.
    #[cfg(feature = "serde")]
    pub fn lift_verification_result(
        &self,
        result: &VerificationResult,
        case_element_id: Option<String>,
    ) -> Verdict {
        match &self.verdict_ctx {
            Some(ctx) => {
                Verdict::from_verification_result_with_evidence(result, ctx, case_element_id)
            }
            None => result.into(),
        }
    }

    fn resolve_requirement_binding(
        &self,
        binding: &RequirementBinding,
        ctx: &EvalContext,
    ) -> Result<Value, BindingResolutionError> {
        match binding {
            RequirementBinding::Literal { value, .. } => Ok(value.clone()),
            RequirementBinding::FeaturePath { path, .. }
            | RequirementBinding::FeaturePathWithFallback { path, .. } => {
                let names: Vec<String> = path.split('.').map(str::to_owned).collect();
                let expr = if names.len() <= 1 {
                    ExprIR::FeatureRef(path.clone())
                } else {
                    ExprIR::FeatureChain(names)
                };
                self.evaluator
                    .eval(&expr, ctx)
                    .or_else(|err| match (binding, err) {
                        (
                            RequirementBinding::FeaturePathWithFallback { fallback, .. },
                            EvaluationError::UndefinedVariable(_),
                        ) => Ok(fallback.clone()),
                        (_, EvaluationError::UndefinedVariable(variable)) => {
                            Err(BindingResolutionError::Undefined {
                                binding: binding.name().to_owned(),
                                source: binding.source_label(),
                                variable,
                            })
                        }
                        (_, other) => Err(BindingResolutionError::Evaluation {
                            binding: binding.name().to_owned(),
                            source: binding.source_label(),
                            message: other.to_string(),
                        }),
                    })
            }
            RequirementBinding::Expression { expr, .. } => {
                self.evaluator.eval(expr, ctx).map_err(|err| match err {
                    EvaluationError::UndefinedVariable(variable) => {
                        BindingResolutionError::Undefined {
                            binding: binding.name().to_owned(),
                            source: binding.source_label(),
                            variable,
                        }
                    }
                    other => BindingResolutionError::Evaluation {
                        binding: binding.name().to_owned(),
                        source: binding.source_label(),
                        message: other.to_string(),
                    },
                })
            }
        }
    }

    fn binding_error_result(
        &self,
        req: &RequirementCheck,
        err: BindingResolutionError,
    ) -> RequirementResult {
        match err {
            BindingResolutionError::Undefined {
                binding,
                source,
                variable,
            } => RequirementResult {
                requirement_id: req.id.clone(),
                source_element_id: req.source_element_id.clone(),
                verdict: VerdictKind::Inconclusive,
                message: format!(
                    "unresolved binding `{}` from `{}`: undefined variable `{}`",
                    binding, source, variable
                ),
                assumptions_met: Vec::new(),
                constraints_met: Vec::new(),
                subrequirement_results: Vec::new(),
            },
            BindingResolutionError::Evaluation {
                binding,
                source,
                message,
            } => RequirementResult {
                requirement_id: req.id.clone(),
                source_element_id: req.source_element_id.clone(),
                verdict: VerdictKind::Error,
                message: format!(
                    "binding `{}` from `{}` evaluation error: {}",
                    binding, source, message
                ),
                assumptions_met: Vec::new(),
                constraints_met: Vec::new(),
                subrequirement_results: Vec::new(),
            },
        }
    }

    /// Execute a verification case.
    ///
    /// The verification pipeline:
    /// 1. Process setup actions, injecting bindings into the context
    /// 2. If any setup action fails, return Error verdict immediately
    /// 3. Check each requirement against the enriched context
    /// 4. Aggregate to overall verdict
    pub fn verify(&self, case: &VerificationCaseIR, ctx: &EvalContext) -> VerificationResult {
        #[cfg(feature = "tracing")]
        tracing::trace!(
            case = %case.name,
            setup_actions = case.setup_actions.len(),
            requirements = case.requirements.len(),
            binding_count = ctx.variables.len(),
            "starting verification case run"
        );

        // Phase 0: the case's own owned-attribute values (KerML: owned
        // features are visible throughout the case body). Seeded FIRST so
        // setup-action outputs win on collision.
        let mut enriched_ctx = ctx.scratch_snapshot();
        for (name, value) in &case.bindings {
            enriched_ctx.set(name.clone(), value.clone());
        }

        // Phase 1: Process setup actions
        for action in &case.setup_actions {
            match action {
                SetupActionResult::Success { bindings } => {
                    for (name, value) in bindings {
                        enriched_ctx.set(name.clone(), value.clone());
                    }
                }
                SetupActionResult::Failure { message } => {
                    #[cfg(feature = "tracing")]
                    tracing::debug!(
                        case = %case.name,
                        error = %message,
                        "verification setup action failed"
                    );
                    return VerificationResult {
                        verdict: VerdictKind::Error,
                        requirement_results: Vec::new(),
                        diagnostics: vec![Diagnostic::error(format!(
                            "setup action failed: {}",
                            message
                        ))],
                    };
                }
            }
        }

        // Phase 2: Check requirements. Evaluated regardless of whether the case
        // models an explicit verdict criterion — the checks are the audit record
        // (§7.24.2 `requirementVerifications`) and stay in the result for display
        // even when a modeled criterion determines the overall verdict.
        //
        // Default (no modeled criterion) — the spec's "commonly" derivation
        // (§8.4.20.1): worst-wins over the checks; a case with NO requirements
        // provides no basis for a determination → Inconclusive, not a vacuous
        // Pass (§7.24.1).
        let mut requirement_results = Vec::new();
        let overall = if case.requirements.is_empty() {
            VerdictKind::Inconclusive
        } else {
            let mut agg = VerdictKind::Pass;
            for req in &case.requirements {
                let result = self.check_requirement(req, &enriched_ctx);
                agg = agg.aggregate(result.verdict);
                requirement_results.push(result);
            }
            agg
        };

        // §8.4.20.1: "this may not always be the desired condition for passing,
        // so the criteria for passing must be modeled explicitly." When the model
        // states a verdict criterion (`return verdict = <expr>`), its value BECOMES
        // the overall verdict — overriding the worst-wins default above (which is
        // spec-correct ONLY as the criterion for a model that states none). The
        // requirement results above are retained for display/audit.
        //
        // Fail-hard: a criterion that cannot be compiled, cannot be evaluated, or
        // evaluates to something that is neither a Boolean nor a VerdictKind is an
        // Error verdict with a diagnostic naming the expression — never a silent
        // fall-back to worst-wins (a modeled criterion that can't be honored is an
        // error, not a shrug).
        if let Some(expr_text) = &case.verdict_expression {
            return self.evaluate_verdict_criterion(expr_text, &enriched_ctx, requirement_results);
        }

        #[cfg(feature = "tracing")]
        tracing::debug!(
            case = %case.name,
            verdict = %overall,
            requirement_results = requirement_results.len(),
            "verification case run finished"
        );

        VerificationResult {
            verdict: overall,
            requirement_results,
            diagnostics: Vec::new(),
        }
    }

    /// Evaluate a modeled verdict criterion (§8.4.20.1) and map its value to a
    /// [`VerdictKind`]. The `requirement_results` computed by the caller are
    /// carried through unchanged (display/audit record); only the overall verdict
    /// comes from the criterion.
    ///
    /// Value mapping:
    /// - `Boolean` — `true` → `Pass`, `false` → `Fail` (the `PassIf` contract,
    ///   `VerificationCases.sysml:70-79`).
    /// - `String` — a `VerdictKind` literal (`pass`/`fail`/`inconclusive`/`error`,
    ///   the library's lowercase enum literals; matched case-insensitively). This
    ///   is also what the built-in `PassIf` returns.
    /// - anything else, or an evaluation/compile failure → `Error` with a
    ///   diagnostic naming the criterion (fail-hard, per the invariant above).
    fn evaluate_verdict_criterion(
        &self,
        expr_text: &str,
        ctx: &EvalContext,
        requirement_results: Vec<RequirementResult>,
    ) -> VerificationResult {
        let error = |message: String| VerificationResult {
            verdict: VerdictKind::Error,
            requirement_results: requirement_results.clone(),
            diagnostics: vec![Diagnostic::error(message)],
        };

        let expr = match compile_simple_expression(expr_text) {
            Ok(e) => e,
            Err(diags) => {
                let detail = diags
                    .iter()
                    .map(|d| d.message.clone())
                    .collect::<Vec<_>>()
                    .join("; ");
                return error(format!(
                    "modeled verdict criterion '{expr_text}' could not be compiled: {detail}"
                ));
            }
        };

        let value = match self.evaluator.eval(&expr, ctx) {
            Ok(v) => v,
            Err(e) => {
                return error(format!(
                    "modeled verdict criterion '{expr_text}' could not be evaluated: {e}"
                ));
            }
        };

        match verdict_from_value(&value) {
            Some(verdict) => VerificationResult {
                verdict,
                requirement_results,
                diagnostics: Vec::new(),
            },
            None => error(format!(
                "modeled verdict criterion '{expr_text}' evaluated to {value:?}, which is neither \
                 a Boolean nor a VerdictKind value"
            )),
        }
    }

    /// Check a single requirement, including nested sub-requirements.
    ///
    /// Requirement semantics (from Requirements.sysml):
    /// - If all assumptions are true, then all constraints must be true.
    /// - If any assumption is false, the requirement is vacuously satisfied.
    /// - All sub-requirements must also pass for the parent to pass.
    /// - An `UndefinedVariable` error produces `Inconclusive` (context incomplete).
    /// - Other evaluation errors produce `Error`.
    pub fn check_requirement(
        &self,
        req: &RequirementCheck,
        ctx: &EvalContext,
    ) -> RequirementResult {
        // Overlay this requirement's per-occurrence bindings onto its own context
        // clone (occurrence-scoped: a sibling verified requirement's bindings for
        // the same attribute name must not leak in). The overlaid context is also
        // passed to sub-requirements so a parent's redefinitions stay in scope.
        //
        // Legacy eager literals are applied first; lazy binding specs are resolved
        // afterward, in order, against the progressively-overlaid check-time
        // context so feature references can consume dynamic run/session outputs.
        let local_ctx;
        let ctx: &EvalContext = if req.bindings.is_empty() && req.binding_specs.is_empty() {
            ctx
        } else {
            let mut overlaid = ctx.scratch_snapshot();
            for (name, value) in &req.bindings {
                overlaid.set(name.clone(), value.clone());
            }
            for binding in &req.binding_specs {
                match self.resolve_requirement_binding(binding, &overlaid) {
                    Ok(value) => overlaid.set(binding.name().to_owned(), value),
                    Err(err) => return self.binding_error_result(req, err),
                }
            }
            local_ctx = overlaid;
            &local_ctx
        };

        #[cfg(feature = "tracing")]
        tracing::trace!(
            requirement_id = %req.id,
            assumptions = req.assumptions.len(),
            constraints = req.constraints.len(),
            subrequirements = req.subrequirements.len(),
            binding_count = ctx.variables.len(),
            "checking requirement"
        );

        // Evaluate assumptions
        let mut assumptions_met = Vec::new();
        for assumption in &req.assumptions {
            match self.evaluator.eval(assumption, ctx) {
                Ok(Value::Bool(b)) => assumptions_met.push(b),
                Ok(_) => {
                    return RequirementResult {
                        requirement_id: req.id.clone(),
                        source_element_id: req.source_element_id.clone(),
                        verdict: VerdictKind::Error,
                        message: "assumption must be boolean".into(),
                        assumptions_met,
                        constraints_met: Vec::new(),
                        subrequirement_results: Vec::new(),
                    };
                }
                Err(EvaluationError::UndefinedVariable(var)) => {
                    return RequirementResult {
                        requirement_id: req.id.clone(),
                        source_element_id: req.source_element_id.clone(),
                        verdict: VerdictKind::Inconclusive,
                        message: format!("undefined variable in assumption: `{}`", var),
                        assumptions_met,
                        constraints_met: Vec::new(),
                        subrequirement_results: Vec::new(),
                    };
                }
                Err(e) => {
                    return RequirementResult {
                        requirement_id: req.id.clone(),
                        source_element_id: req.source_element_id.clone(),
                        verdict: VerdictKind::Error,
                        message: format!("assumption evaluation error: {}", e),
                        assumptions_met,
                        constraints_met: Vec::new(),
                        subrequirement_results: Vec::new(),
                    };
                }
            }
        }

        // If any assumption is false, requirement is vacuously satisfied
        if assumptions_met.iter().any(|a| !a) {
            #[cfg(feature = "tracing")]
            tracing::trace!(
                requirement_id = %req.id,
                "requirement vacuously satisfied (assumption not met)"
            );
            return RequirementResult {
                requirement_id: req.id.clone(),
                source_element_id: req.source_element_id.clone(),
                verdict: VerdictKind::Pass,
                message: "vacuously satisfied (assumption not met)".into(),
                assumptions_met,
                constraints_met: Vec::new(),
                subrequirement_results: Vec::new(),
            };
        }

        // If there are compile errors and no valid constraints, this is an error
        if req.constraints.is_empty() && !req.compile_errors.is_empty() {
            return RequirementResult {
                requirement_id: req.id.clone(),
                source_element_id: req.source_element_id.clone(),
                verdict: VerdictKind::Error,
                message: format!(
                    "all constraints failed to compile: {}",
                    req.compile_errors.join("; ")
                ),
                assumptions_met,
                constraints_met: Vec::new(),
                subrequirement_results: Vec::new(),
            };
        }

        // SysML §8.4.20.1 / §7.24.1: pass criteria must be modeled explicitly. A
        // requirement (assumptions met / none) with NO modeled criteria — no
        // constraints AND no subrequirements — provides no basis for a
        // determination, so its verdict is Inconclusive, not a vacuous Pass. This
        // is distinct from the unmet-assumption vacuous satisfaction above (a true
        // implication with a false premise), and from a constraint that evaluates
        // to Inconclusive on an unbound feature (handled in the loop below).
        if req.constraints.is_empty() && req.subrequirements.is_empty() {
            return RequirementResult {
                requirement_id: req.id.clone(),
                source_element_id: req.source_element_id.clone(),
                verdict: VerdictKind::Inconclusive,
                message: "no modeled pass criteria — determination cannot be made".into(),
                assumptions_met,
                constraints_met: Vec::new(),
                subrequirement_results: Vec::new(),
            };
        }

        // Evaluate constraints
        let mut constraints_met = Vec::new();
        let mut constraint_verdict = VerdictKind::Pass;
        for constraint in &req.constraints {
            match self.evaluator.eval(constraint, ctx) {
                Ok(Value::Bool(b)) => {
                    constraints_met.push(b);
                    if !b {
                        constraint_verdict = constraint_verdict.aggregate(VerdictKind::Fail);
                    }
                }
                Ok(_) => {
                    return RequirementResult {
                        requirement_id: req.id.clone(),
                        source_element_id: req.source_element_id.clone(),
                        verdict: VerdictKind::Error,
                        message: "constraint must be boolean".into(),
                        assumptions_met,
                        constraints_met,
                        subrequirement_results: Vec::new(),
                    };
                }
                Err(EvaluationError::UndefinedVariable(var)) => {
                    return RequirementResult {
                        requirement_id: req.id.clone(),
                        source_element_id: req.source_element_id.clone(),
                        verdict: VerdictKind::Inconclusive,
                        message: format!("undefined variable in constraint: `{}`", var),
                        assumptions_met,
                        constraints_met,
                        subrequirement_results: Vec::new(),
                    };
                }
                Err(e) => {
                    return RequirementResult {
                        requirement_id: req.id.clone(),
                        source_element_id: req.source_element_id.clone(),
                        verdict: VerdictKind::Error,
                        message: format!("constraint evaluation error: {}", e),
                        assumptions_met,
                        constraints_met,
                        subrequirement_results: Vec::new(),
                    };
                }
            }
        }

        // Evaluate sub-requirements
        let mut subrequirement_results = Vec::new();
        let mut sub_verdict = VerdictKind::Pass;
        for subreq in &req.subrequirements {
            let sub_result = self.check_requirement(subreq, ctx);
            sub_verdict = sub_verdict.aggregate(sub_result.verdict);
            subrequirement_results.push(sub_result);
        }

        // Aggregate: own constraints + sub-requirement verdicts
        let verdict = constraint_verdict.aggregate(sub_verdict);

        let mut message = match verdict {
            VerdictKind::Pass => "all constraints satisfied".into(),
            _ => {
                let mut parts = Vec::new();
                let failed_constraints: Vec<_> = constraints_met
                    .iter()
                    .enumerate()
                    .filter(|(_, met)| !**met)
                    .map(|(i, _)| format!("constraint[{}]", i))
                    .collect();
                if !failed_constraints.is_empty() {
                    parts.push(format!("failed: {}", failed_constraints.join(", ")));
                }
                let failed_subs: Vec<_> = subrequirement_results
                    .iter()
                    .filter(|r| !r.verdict.is_pass())
                    .map(|r| r.requirement_id.clone())
                    .collect();
                if !failed_subs.is_empty() {
                    parts.push(format!(
                        "sub-requirements not satisfied: {}",
                        failed_subs.join(", ")
                    ));
                }
                if parts.is_empty() {
                    format!("{}", verdict)
                } else {
                    parts.join("; ")
                }
            }
        };

        // Annotation marker (one home for every verdict surface — verify,
        // sessions.verify, CLI, MCP, app overlay): on a non-pass verdict, append
        // the FIRST SENTENCE of the requirement's own modeled text so a bare `fail`
        // carries its WHY. This is how a model marks an EXPECTED fail as a tracked
        // KNOWN GAP (e.g. a fixture's NuisanceTripFloor doc) rather than a regression: the
        // marker lives in the requirement's text, and every consumer of {verdict,
        // message} sees it without a bespoke channel. Only the lead sentence rides
        // the message (overlay readability); the FULL body stays available on the
        // `requirement_text` response field. Pass verdicts keep their plain "all
        // constraints satisfied" (no ripple on the green path).
        if verdict != VerdictKind::Pass {
            if let Some(text) = req.text.as_deref().map(marker_lead_sentence) {
                if !text.is_empty() {
                    message = format!("{message} — {text}");
                }
            }
        }

        #[cfg(feature = "tracing")]
        tracing::trace!(
            requirement_id = %req.id,
            verdict = %verdict,
            "requirement check complete"
        );

        RequirementResult {
            requirement_id: req.id.clone(),
            source_element_id: req.source_element_id.clone(),
            verdict,
            message,
            assumptions_met,
            constraints_met,
            subrequirement_results,
        }
    }
}

impl Default for VerificationRunner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::cases::test_support::*;
    use crate::expressions::{BinOp, EvalContext, ExprIR};
    use sysml_core::Value;

    #[test]
    fn simple_verification_pass() {
        let case = simple_case(
            "vc1",
            vec![simple_req(
                "req1",
                vec![],
                vec![ExprIR::BinaryOp {
                    op: BinOp::LessThan,
                    left: Box::new(ExprIR::FeatureRef("x".into())),
                    right: Box::new(ExprIR::LiteralInt(100)),
                }],
            )],
        );

        let mut ctx = EvalContext::new();
        ctx.set("x", Value::Int(50));

        let runner = VerificationRunner::new();
        let result = runner.verify(&case, &ctx);
        assert_eq!(result.verdict, VerdictKind::Pass);
    }

    #[test]
    fn lazy_feature_path_binding_resolves_against_check_time_context() {
        let mut req = simple_req(
            "req-lazy-path",
            vec![],
            vec![ExprIR::BinaryOp {
                op: BinOp::LessThan,
                left: Box::new(ExprIR::FeatureChain(vec!["w".into(), "mass".into()])),
                right: Box::new(ExprIR::LiteralInt(10)),
            }],
        );
        req.binding_specs.push(RequirementBinding::FeaturePath {
            name: "w.mass".into(),
            path: "massRun.massResult".into(),
        });

        let mut ctx = EvalContext::new();
        ctx.set("massRun.massResult", Value::Int(5));

        let runner = VerificationRunner::new();
        let result = runner.check_requirement(&req, &ctx);
        assert_eq!(result.verdict, VerdictKind::Pass);
    }

    #[test]
    fn computed_binding_expression_resolves_at_check_time() {
        let mut req = simple_req(
            "req-computed-binding",
            vec![],
            vec![ExprIR::BinaryOp {
                op: BinOp::Equal,
                left: Box::new(ExprIR::FeatureChain(vec!["w".into(), "mass".into()])),
                right: Box::new(ExprIR::LiteralInt(5)),
            }],
        );
        req.binding_specs.push(RequirementBinding::Expression {
            name: "w.mass".into(),
            expr: ExprIR::BinaryOp {
                op: BinOp::Add,
                left: Box::new(ExprIR::FeatureRef("measured".into())),
                right: Box::new(ExprIR::LiteralInt(2)),
            },
        });

        let mut ctx = EvalContext::new();
        ctx.set("measured", Value::Int(3));

        let runner = VerificationRunner::new();
        let result = runner.check_requirement(&req, &ctx);
        assert_eq!(result.verdict, VerdictKind::Pass);
    }

    #[test]
    fn unresolved_lazy_binding_is_inconclusive_not_fake_pass_or_fail() {
        let mut req = simple_req(
            "req-unresolved-binding",
            vec![],
            vec![ExprIR::BinaryOp {
                op: BinOp::LessThan,
                left: Box::new(ExprIR::FeatureChain(vec!["w".into(), "mass".into()])),
                right: Box::new(ExprIR::LiteralInt(10)),
            }],
        );
        req.binding_specs.push(RequirementBinding::FeaturePath {
            name: "w.mass".into(),
            path: "missingRun.massResult".into(),
        });

        let runner = VerificationRunner::new();
        let result = runner.check_requirement(&req, &EvalContext::new());
        assert_eq!(result.verdict, VerdictKind::Inconclusive);
        assert!(result.message.contains("unresolved binding `w.mass`"));
    }

    #[test]
    fn per_occurrence_binding_overrides_are_isolated() {
        let mut req_a = simple_req(
            "req-a",
            vec![],
            vec![ExprIR::BinaryOp {
                op: BinOp::Equal,
                left: Box::new(ExprIR::FeatureRef("x".into())),
                right: Box::new(ExprIR::LiteralInt(1)),
            }],
        );
        req_a.binding_specs.push(RequirementBinding::Literal {
            name: "x".into(),
            value: Value::Int(1),
        });

        let mut req_b = simple_req(
            "req-b",
            vec![],
            vec![ExprIR::BinaryOp {
                op: BinOp::Equal,
                left: Box::new(ExprIR::FeatureRef("x".into())),
                right: Box::new(ExprIR::LiteralInt(2)),
            }],
        );
        req_b.binding_specs.push(RequirementBinding::Literal {
            name: "x".into(),
            value: Value::Int(2),
        });

        let runner = VerificationRunner::new();
        let case = simple_case("vc-isolated-bindings", vec![req_a, req_b]);
        let result = runner.verify(&case, &EvalContext::new());
        assert_eq!(result.verdict, VerdictKind::Pass);
        assert_eq!(result.requirement_results[0].verdict, VerdictKind::Pass);
        assert_eq!(result.requirement_results[1].verdict, VerdictKind::Pass);
    }

    #[test]
    fn simple_verification_fail() {
        let case = simple_case(
            "vc1",
            vec![simple_req(
                "req1",
                vec![],
                vec![ExprIR::BinaryOp {
                    op: BinOp::LessThan,
                    left: Box::new(ExprIR::FeatureRef("x".into())),
                    right: Box::new(ExprIR::LiteralInt(100)),
                }],
            )],
        );

        let mut ctx = EvalContext::new();
        ctx.set("x", Value::Int(150));

        let runner = VerificationRunner::new();
        let result = runner.verify(&case, &ctx);
        assert_eq!(result.verdict, VerdictKind::Fail);
    }

    #[test]
    fn vacuous_satisfaction() {
        let case = simple_case(
            "vc1",
            vec![simple_req(
                "req1",
                vec![ExprIR::LiteralBool(false)], // Assumption is false
                vec![ExprIR::LiteralBool(false)], // Constraint would fail
            )],
        );

        let ctx = EvalContext::new();
        let runner = VerificationRunner::new();
        let result = runner.verify(&case, &ctx);
        // Vacuously satisfied because assumption is false
        assert_eq!(result.verdict, VerdictKind::Pass);
    }

    #[test]
    fn multiple_requirements_aggregate() {
        let case = simple_case(
            "vc1",
            vec![
                simple_req("req1", vec![], vec![ExprIR::LiteralBool(true)]),
                simple_req("req2", vec![], vec![ExprIR::LiteralBool(false)]),
            ],
        );

        let ctx = EvalContext::new();
        let runner = VerificationRunner::new();
        let result = runner.verify(&case, &ctx);
        // One pass + one fail = overall fail
        assert_eq!(result.verdict, VerdictKind::Fail);
        assert_eq!(result.requirement_results[0].verdict, VerdictKind::Pass);
        assert_eq!(result.requirement_results[1].verdict, VerdictKind::Fail);
    }

    #[test]
    fn nested_subrequirements_evaluated() {
        // A parent requirement with two sub-requirements that must all pass
        let case = simple_case(
            "vc-nested",
            vec![RequirementCheck {
                id: "req-parent".into(),
                source_element_id: None,
                text: Some("Vehicle safety".into()),
                assumptions: vec![],
                constraints: vec![ExprIR::LiteralBool(true)], // Parent's own constraint passes
                constraint_element_ids: vec![None],
                compile_errors: Vec::new(),
                bindings: Vec::new(),
                binding_specs: Vec::new(),
                subrequirements: vec![
                    simple_req(
                        "req-child-1",
                        vec![],
                        vec![ExprIR::BinaryOp {
                            op: BinOp::LessThan,
                            left: Box::new(ExprIR::FeatureRef("speed".into())),
                            right: Box::new(ExprIR::LiteralInt(200)),
                        }],
                    ),
                    simple_req(
                        "req-child-2",
                        vec![],
                        vec![ExprIR::BinaryOp {
                            op: BinOp::GreaterThan,
                            left: Box::new(ExprIR::FeatureRef("fuel".into())),
                            right: Box::new(ExprIR::LiteralInt(0)),
                        }],
                    ),
                ],
            }],
        );

        let mut ctx = EvalContext::new();
        ctx.set("speed", Value::Int(120));
        ctx.set("fuel", Value::Int(50));

        let runner = VerificationRunner::new();
        let result = runner.verify(&case, &ctx);
        assert_eq!(result.verdict, VerdictKind::Pass);
        assert_eq!(
            result.requirement_results[0].subrequirement_results.len(),
            2
        );
        assert_eq!(
            result.requirement_results[0].subrequirement_results[0].verdict,
            VerdictKind::Pass
        );
        assert_eq!(
            result.requirement_results[0].subrequirement_results[1].verdict,
            VerdictKind::Pass
        );

        // Now with a failing sub-requirement (fuel = 0)
        let mut ctx2 = EvalContext::new();
        ctx2.set("speed", Value::Int(120));
        ctx2.set("fuel", Value::Int(0)); // fuel > 0 will fail
        let result2 = runner.verify(&case, &ctx2);
        assert_eq!(result2.verdict, VerdictKind::Fail);
        assert_eq!(
            result2.requirement_results[0].subrequirement_results[1].verdict,
            VerdictKind::Fail
        );
    }

    #[test]
    fn mixed_assumptions_and_constraints() {
        // Case with multiple requirements: some with assumptions, some without
        let case = simple_case(
            "vc-mixed",
            vec![
                // Requirement with assumption that is false -> vacuously satisfied
                simple_req(
                    "req-vacuous",
                    vec![ExprIR::LiteralBool(false)],
                    vec![ExprIR::LiteralBool(false)], // Would fail if evaluated
                ),
                // Requirement with no assumptions, constraint passes
                simple_req("req-direct", vec![], vec![ExprIR::LiteralBool(true)]),
                // Requirement with true assumption, constraint must pass
                simple_req(
                    "req-guarded",
                    vec![ExprIR::BinaryOp {
                        op: BinOp::GreaterThan,
                        left: Box::new(ExprIR::FeatureRef("temp".into())),
                        right: Box::new(ExprIR::LiteralInt(0)),
                    }],
                    vec![ExprIR::BinaryOp {
                        op: BinOp::LessThan,
                        left: Box::new(ExprIR::FeatureRef("temp".into())),
                        right: Box::new(ExprIR::LiteralInt(100)),
                    }],
                ),
            ],
        );

        let mut ctx = EvalContext::new();
        ctx.set("temp", Value::Int(50));

        let runner = VerificationRunner::new();
        let result = runner.verify(&case, &ctx);
        assert_eq!(result.verdict, VerdictKind::Pass);
        // First is vacuously satisfied
        assert_eq!(result.requirement_results[0].verdict, VerdictKind::Pass);
        assert!(result.requirement_results[0].message.contains("vacuously"));
        // Second is directly satisfied
        assert_eq!(result.requirement_results[1].verdict, VerdictKind::Pass);
        // Third is guarded and passes
        assert_eq!(result.requirement_results[2].verdict, VerdictKind::Pass);
    }

    #[test]
    fn inconclusive_on_undetermined() {
        // A constraint references an undefined variable -> Inconclusive
        let case = simple_case(
            "vc-inconclusive",
            vec![simple_req(
                "req1",
                vec![],
                vec![ExprIR::BinaryOp {
                    op: BinOp::LessThan,
                    left: Box::new(ExprIR::FeatureRef("unknown_var".into())),
                    right: Box::new(ExprIR::LiteralInt(100)),
                }],
            )],
        );

        let ctx = EvalContext::new(); // No variables set
        let runner = VerificationRunner::new();
        let result = runner.verify(&case, &ctx);
        assert_eq!(result.verdict, VerdictKind::Inconclusive);
        assert!(result.requirement_results[0]
            .message
            .contains("undefined variable"));
    }

    #[test]
    fn error_on_eval_failure() {
        // A constraint that produces a type error (not boolean) -> Error
        let case = simple_case(
            "vc-error",
            vec![simple_req(
                "req1",
                vec![],
                // Evaluates to an integer, not a boolean
                vec![ExprIR::BinaryOp {
                    op: BinOp::Add,
                    left: Box::new(ExprIR::LiteralInt(1)),
                    right: Box::new(ExprIR::LiteralInt(2)),
                }],
            )],
        );

        let ctx = EvalContext::new();
        let runner = VerificationRunner::new();
        let result = runner.verify(&case, &ctx);
        assert_eq!(result.verdict, VerdictKind::Error);
        assert!(result.requirement_results[0]
            .message
            .contains("constraint must be boolean"));
    }

    #[test]
    fn verification_runs_actions_then_checks() {
        // Setup actions inject bindings, then requirements check them
        let case = VerificationCaseIR {
            id: "vc-action".into(),
            name: "Action-driven".into(),
            subject: None,
            setup_actions: vec![SetupActionResult::Success {
                bindings: vec![
                    ("measured_speed".into(), Value::Int(85)),
                    ("measured_temp".into(), Value::Int(72)),
                ],
            }],
            requirements: vec![simple_req(
                "req-speed",
                vec![],
                vec![ExprIR::BinaryOp {
                    op: BinOp::LessThan,
                    left: Box::new(ExprIR::FeatureRef("measured_speed".into())),
                    right: Box::new(ExprIR::LiteralInt(100)),
                }],
            )],
            sub_cases: Vec::new(),
            verdict_expression: None,
            bindings: Vec::new(),
        };

        let ctx = EvalContext::new(); // No pre-existing bindings
        let runner = VerificationRunner::new();
        let result = runner.verify(&case, &ctx);
        // Action injected measured_speed=85, which is < 100
        assert_eq!(result.verdict, VerdictKind::Pass);
    }

    #[test]
    fn action_output_available_to_requirements() {
        // Multiple setup actions contribute bindings, all available to requirements
        let case = VerificationCaseIR {
            id: "vc-multi-action".into(),
            name: "Multi-action".into(),
            subject: None,
            setup_actions: vec![
                SetupActionResult::Success {
                    bindings: vec![("sensor_a".into(), Value::Int(10))],
                },
                SetupActionResult::Success {
                    bindings: vec![("sensor_b".into(), Value::Int(20))],
                },
            ],
            requirements: vec![
                simple_req(
                    "req-a",
                    vec![],
                    vec![ExprIR::BinaryOp {
                        op: BinOp::GreaterThan,
                        left: Box::new(ExprIR::FeatureRef("sensor_a".into())),
                        right: Box::new(ExprIR::LiteralInt(0)),
                    }],
                ),
                simple_req(
                    "req-b",
                    vec![],
                    vec![ExprIR::BinaryOp {
                        op: BinOp::GreaterThan,
                        left: Box::new(ExprIR::FeatureRef("sensor_b".into())),
                        right: Box::new(ExprIR::LiteralInt(0)),
                    }],
                ),
            ],
            sub_cases: Vec::new(),
            verdict_expression: None,
            bindings: Vec::new(),
        };

        let ctx = EvalContext::new();
        let runner = VerificationRunner::new();
        let result = runner.verify(&case, &ctx);
        assert_eq!(result.verdict, VerdictKind::Pass);
        assert_eq!(result.requirement_results.len(), 2);
        assert_eq!(result.requirement_results[0].verdict, VerdictKind::Pass);
        assert_eq!(result.requirement_results[1].verdict, VerdictKind::Pass);
    }

    #[test]
    fn action_failure_produces_error_verdict() {
        // A setup action failure should produce Error verdict immediately
        let case = VerificationCaseIR {
            id: "vc-fail-action".into(),
            name: "Failed action".into(),
            subject: None,
            setup_actions: vec![
                SetupActionResult::Success {
                    bindings: vec![("ok_var".into(), Value::Int(1))],
                },
                SetupActionResult::Failure {
                    message: "sensor initialization timeout".into(),
                },
            ],
            requirements: vec![simple_req(
                "req1",
                vec![],
                vec![ExprIR::LiteralBool(true)], // Would pass if reached
            )],
            sub_cases: Vec::new(),
            verdict_expression: None,
            bindings: Vec::new(),
        };

        let ctx = EvalContext::new();
        let runner = VerificationRunner::new();
        let result = runner.verify(&case, &ctx);
        assert_eq!(result.verdict, VerdictKind::Error);
        // No requirement results because setup failed
        assert!(result.requirement_results.is_empty());
        assert!(!result.diagnostics.is_empty());
        assert!(result.diagnostics[0]
            .message
            .contains("sensor initialization timeout"));
    }

    #[test]
    fn verification_case_full_pipeline() {
        // End-to-end verification case that combines setup actions (simulating
        // action execution) with requirement constraint checking (using the
        // expression evaluator from sysml-run-expressions).
        //
        // Pipeline:
        //   1. Setup action sets speed = 85 in context
        //   2. Requirement: speed < 100
        //   3. Verdict: Pass (85 < 100)
        let case = VerificationCaseIR {
            id: "vc-full-pipeline".into(),
            name: "Speed Verification".into(),
            subject: Some("vehicle".into()),
            setup_actions: vec![SetupActionResult::Success {
                bindings: vec![("speed".into(), Value::Int(85))],
            }],
            requirements: vec![RequirementCheck {
                id: "req-speed-limit".into(),
                source_element_id: None,
                text: Some("Vehicle speed must be under 100".into()),
                assumptions: vec![],
                constraints: vec![ExprIR::BinaryOp {
                    op: BinOp::LessThan,
                    left: Box::new(ExprIR::FeatureRef("speed".into())),
                    right: Box::new(ExprIR::LiteralInt(100)),
                }],
                constraint_element_ids: vec![None],
                compile_errors: Vec::new(),
                subrequirements: vec![],
                bindings: Vec::new(),
                binding_specs: Vec::new(),
            }],
            sub_cases: Vec::new(),
            verdict_expression: None,
            bindings: Vec::new(),
        };

        // Start with an empty context — the setup action provides the binding
        let ctx = EvalContext::new();
        let runner = VerificationRunner::new();
        let result = runner.verify(&case, &ctx);

        // The setup action injects speed=85, which satisfies speed < 100
        assert_eq!(result.verdict, VerdictKind::Pass);
        assert_eq!(result.requirement_results.len(), 1);
        assert_eq!(result.requirement_results[0].verdict, VerdictKind::Pass);
        assert!(result.diagnostics.is_empty());

        // Now verify failure: setup action sets speed = 120, which violates speed < 100
        let failing_case = VerificationCaseIR {
            id: "vc-full-pipeline-fail".into(),
            name: "Speed Verification (fail)".into(),
            subject: Some("vehicle".into()),
            setup_actions: vec![SetupActionResult::Success {
                bindings: vec![("speed".into(), Value::Int(120))],
            }],
            requirements: vec![RequirementCheck {
                id: "req-speed-limit".into(),
                source_element_id: None,
                text: Some("Vehicle speed must be under 100".into()),
                assumptions: vec![],
                constraints: vec![ExprIR::BinaryOp {
                    op: BinOp::LessThan,
                    left: Box::new(ExprIR::FeatureRef("speed".into())),
                    right: Box::new(ExprIR::LiteralInt(100)),
                }],
                constraint_element_ids: vec![None],
                compile_errors: Vec::new(),
                subrequirements: vec![],
                bindings: Vec::new(),
                binding_specs: Vec::new(),
            }],
            sub_cases: Vec::new(),
            verdict_expression: None,
            bindings: Vec::new(),
        };

        let fail_result = runner.verify(&failing_case, &ctx);
        assert_eq!(fail_result.verdict, VerdictKind::Fail);
        assert_eq!(
            fail_result.requirement_results[0].verdict,
            VerdictKind::Fail
        );
    }

    #[test]
    fn invalid_expression_produces_error_not_pass() {
        // A requirement whose only constraint failed to compile should produce
        // Error, not vacuously Pass (the false-pass bug).
        let req = RequirementCheck {
            id: "req-broken".into(),
            source_element_id: None,
            text: Some("Should not pass".into()),
            assumptions: vec![],
            constraints: vec![], // No valid constraints
            constraint_element_ids: vec![],
            compile_errors: vec!["parse error: unexpected token `<<<`".into()],
            subrequirements: vec![],
            bindings: Vec::new(),
            binding_specs: Vec::new(),
        };

        let case = simple_case("vc-broken", vec![req]);
        let ctx = EvalContext::new();
        let runner = VerificationRunner::new();
        let result = runner.verify(&case, &ctx);
        assert_eq!(result.verdict, VerdictKind::Error);
        assert!(result.requirement_results[0]
            .message
            .contains("all constraints failed to compile"));
    }

    #[test]
    fn partial_invalid_constraints_still_check_valid_ones() {
        // A requirement with some valid and some invalid constraints should
        // still check the valid ones (not short-circuit to Error).
        let req = RequirementCheck {
            id: "req-partial".into(),
            source_element_id: None,
            text: None,
            assumptions: vec![],
            constraints: vec![ExprIR::LiteralBool(true)], // One valid constraint
            constraint_element_ids: vec![None],
            compile_errors: vec!["parse error: bad expression".into()], // One error
            subrequirements: vec![],
            bindings: Vec::new(),
            binding_specs: Vec::new(),
        };

        let case = simple_case("vc-partial", vec![req]);
        let ctx = EvalContext::new();
        let runner = VerificationRunner::new();
        let result = runner.verify(&case, &ctx);
        // Has valid constraints so should not short-circuit; the valid one passes
        assert_eq!(result.verdict, VerdictKind::Pass);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn runtime_coupled_verify_populates_evidence_on_fail() {
        // R3.5 runtime-coupled site: runner with context produces verdicts
        // whose evidence points at (session_id, tick, source_element_id) — the
        // real model element id, NOT the (non-unique) requirement name.
        let case = VerificationCaseIR {
            id: "vc-runtime".into(),
            name: "runtime-case".into(),
            subject: None,
            setup_actions: vec![],
            requirements: vec![RequirementCheck {
                id: "req-fail".into(),
                text: None,
                assumptions: vec![],
                constraints: vec![ExprIR::LiteralBool(false)],
                compile_errors: vec![],
                subrequirements: vec![],
                source_element_id: Some("req-fail-element-id".into()),
                constraint_element_ids: vec![None],
                bindings: vec![],
                binding_specs: vec![],
            }],
            sub_cases: vec![],
            verdict_expression: None,
            bindings: Vec::new(),
        };

        let ctx = EvalContext::new();
        let verdict_ctx = VerdictContext::new("runtime-session-1", 13);
        let runner = VerificationRunner::with_context(verdict_ctx.clone());
        let result = runner.verify(&case, &ctx);
        assert_eq!(result.verdict, VerdictKind::Fail);

        // Lift each requirement result into a Verdict; evidence must be
        // populated with the runner's session + tick.
        let verdicts: Vec<Verdict> = result
            .requirement_results
            .iter()
            .map(|r| runner.lift_requirement_result(r))
            .collect();
        assert_eq!(verdicts.len(), 1);
        let ev = verdicts[0]
            .evidence
            .as_ref()
            .expect("evidence populated on runtime-coupled fail");
        assert_eq!(ev.session_id, "runtime-session-1");
        assert_eq!(ev.tick, 13);
        // Evidence deep-links via the threaded source element id, not the name.
        assert_eq!(ev.element_id.as_deref(), Some("req-fail-element-id"));

        // Aggregate verdict carries evidence too, bound to the case element.
        let agg = runner.lift_verification_result(&result, Some("vc-runtime".into()));
        let agg_ev = agg.evidence.as_ref().expect("aggregate evidence populated");
        assert_eq!(agg_ev.session_id, "runtime-session-1");
        assert_eq!(agg_ev.tick, 13);
        assert_eq!(agg_ev.element_id.as_deref(), Some("vc-runtime"));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn static_verify_leaves_evidence_none_legitimately() {
        // Static (no-session) verification: Verdict.evidence must stay None.
        // This is the legitimate no-evidence path — code-lens / pre-flight
        // callers have no live session to deep-link to.
        let case = VerificationCaseIR {
            id: "vc-static".into(),
            name: "static-case".into(),
            subject: None,
            setup_actions: vec![],
            requirements: vec![RequirementCheck {
                id: "req-static".into(),
                text: None,
                assumptions: vec![],
                constraints: vec![ExprIR::LiteralBool(true)],
                compile_errors: vec![],
                subrequirements: vec![],
                source_element_id: None,
                constraint_element_ids: vec![None],
                bindings: vec![],
                binding_specs: vec![],
            }],
            sub_cases: vec![],
            verdict_expression: None,
            bindings: Vec::new(),
        };

        let ctx = EvalContext::new();
        let runner = VerificationRunner::new();
        assert!(runner.verdict_ctx().is_none());
        let result = runner.verify(&case, &ctx);

        let verdict = runner.lift_requirement_result(&result.requirement_results[0]);
        assert!(
            verdict.evidence.is_none(),
            "static verify must leave evidence = None"
        );
        let agg = runner.lift_verification_result(&result, Some("vc-static".into()));
        assert!(
            agg.evidence.is_none(),
            "static aggregate must leave evidence = None"
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn verification_case_execute_with_context_threads_evidence() {
        // execute_with_context should propagate the session context into
        // nested sub-cases' results as well.
        let sub = VerificationCaseIR {
            id: "sub-1".into(),
            name: "sub-1".into(),
            subject: None,
            setup_actions: vec![],
            requirements: vec![RequirementCheck {
                id: "sub-req".into(),
                text: None,
                assumptions: vec![],
                constraints: vec![ExprIR::LiteralBool(false)],
                compile_errors: vec![],
                subrequirements: vec![],
                source_element_id: None,
                constraint_element_ids: vec![None],
                bindings: vec![],
                binding_specs: vec![],
            }],
            sub_cases: vec![],
            verdict_expression: None,
            bindings: Vec::new(),
        };
        let case = VerificationCaseIR {
            id: "parent".into(),
            name: "parent".into(),
            subject: None,
            setup_actions: vec![],
            requirements: vec![],
            sub_cases: vec![sub],
            verdict_expression: None,
            bindings: Vec::new(),
        };

        let ctx = EvalContext::new();
        let verdict_ctx = VerdictContext::new("sess-nested", 5);
        let result = case.execute_with_context(&ctx, verdict_ctx.clone());

        // aggregate outcome should fail because the inner constraint is false
        assert_eq!(result.verdict, VerdictKind::Fail);

        // Lift the sub-requirement's result through a runner bound to the
        // same context — evidence should be present.
        let runner = VerificationRunner::with_context(verdict_ctx);
        let rr = result
            .requirement_results
            .first()
            .expect("sub result propagated");
        let lifted = runner.lift_requirement_result(rr);
        let ev = lifted.evidence.as_ref().expect("nested evidence populated");
        assert_eq!(ev.session_id, "sess-nested");
        assert_eq!(ev.tick, 5);
    }

    #[test]
    fn verify_chain_produces_inconclusive_without_binding() {
        // With no value bound for `temp`, the runner returns an Inconclusive
        // verdict (UndefinedVariable path) — NOT an Error. Before B14 the
        // whole case came back as Error "all constraints failed to compile".
        let (graph, _vc_id) = build_verify_chain_graph();
        let ir = compile_verification_case("BrewTempTest", &graph).unwrap();

        let runner = VerificationRunner::new();
        let ctx = EvalContext::new();
        let result = runner.verify(&ir, &ctx);

        assert!(
            matches!(result.verdict, VerdictKind::Inconclusive),
            "expected Inconclusive verdict (temp unbound), got {:?}",
            result.verdict,
        );
    }

    #[test]
    fn verify_chain_passes_when_binding_satisfies_constraint() {
        // With `temp = 92` in the context, the constraint 90<=temp<=96 holds,
        // so the aggregated verdict is Pass.
        let (graph, _vc_id) = build_verify_chain_graph();
        let ir = compile_verification_case("BrewTempTest", &graph).unwrap();

        let runner = VerificationRunner::new();
        let mut ctx = EvalContext::new();
        ctx.set("temp".to_owned(), Value::Int(92));
        let result = runner.verify(&ir, &ctx);

        assert!(
            matches!(result.verdict, VerdictKind::Pass),
            "expected Pass when temp=92 satisfies 90<=temp<=96, got {:?}",
            result.verdict,
        );
    }
}
