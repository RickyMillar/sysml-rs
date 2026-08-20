//! Compiled case IR types (verification / use-case / analysis) and setup-action results.

use crate::expressions::{compile_simple_expression, EvalContext, ExpressionEvaluator};
use crate::solver_plugin::{SolverError, SolverParam, SolverResult};
use crate::solver_registry::SolverRegistry;
use crate::ConstraintIR;
use sysml_core::Value;
use sysml_span::Diagnostic;

#[allow(unused_imports)]
use super::*;

// ---------------------------------------------------------------------------
// Action setup for verification (S2b)
// ---------------------------------------------------------------------------

/// Outcome of a setup action executed before requirement checking.
///
/// In a real system, actions would be executed via sysml-run-actions. Here
/// we accept pre-computed results so that the verification runner can
/// incorporate action outputs without depending on the action runner crate.
#[derive(Debug, Clone)]
pub enum SetupActionResult {
    /// The action succeeded, producing variable bindings.
    Success {
        /// Variable bindings produced by the action (injected into EvalContext).
        bindings: Vec<(String, Value)>,
    },
    /// The action failed with an error message.
    Failure {
        /// Description of the failure.
        message: String,
    },
}

// ---------------------------------------------------------------------------
// Case IR types
// ---------------------------------------------------------------------------

/// Compiled verification case.
#[derive(Debug, Clone)]
pub struct VerificationCaseIR {
    /// Case identifier.
    pub id: String,
    /// Case name.
    pub name: String,
    /// Subject under verification.
    pub subject: Option<String>,
    /// Setup action results to inject before checking requirements.
    ///
    /// These simulate pre-executed verification actions whose outputs feed
    /// into the requirement evaluation context.
    pub setup_actions: Vec<SetupActionResult>,
    /// Requirements to verify.
    pub requirements: Vec<RequirementCheck>,
    /// Sub-verification cases (nested verification within this case).
    pub sub_cases: Vec<VerificationCaseIR>,
    /// Modeled verdict criterion (§8.4.20.1): the expression text of the case's
    /// `return verdict = <expr>` result feature, when the model states its pass
    /// criteria explicitly (VerificationCases.sysml:22 `return verdict :
    /// VerdictKind :>> result`; the `PassIf` helper is the canonical form,
    /// :70-79). When present, [`VerificationRunner::verify`] evaluates it and its
    /// value BECOMES the overall verdict — overriding the "commonly" worst-wins
    /// default, which the spec permits only as the criterion a model states when
    /// it states none of its own. When `None`, the default (worst-wins over
    /// requirement checks; empty → Inconclusive) is unchanged.
    ///
    /// Stored as a display string and compiled/evaluated at verify time, mirroring
    /// [`AnalysisCaseIR::result_expression`] — one pretty-print → compile → eval
    /// pattern for both case kinds, no divergent shape.
    pub verdict_expression: Option<String>,
    /// Literal values of the case's OWN owned attributes (`attribute
    /// declaredMarginDb : Real = 2.5;` in the case body). KerML namespace
    /// semantics: a case's owned features are visible throughout its body, so
    /// the modeled verdict criterion (and requirement checks) evaluate with
    /// them in scope. Collected at compile time by the same helper subject
    /// attributes use ([`collect_occurrence_attribute_values`]); seeded into
    /// the verify context BEFORE setup actions (setup output wins on
    /// collision).
    pub bindings: Vec<(String, Value)>,
}

impl VerificationCaseIR {
    /// Execute this verification case, including any nested sub-cases.
    ///
    /// Uses a `VerificationRunner` internally to check all requirements,
    /// then recursively executes sub-cases and merges their results.
    ///
    /// Evidence is not attached — this is the static entry point used by
    /// code-lenses and pre-flight checks. For runtime-coupled verification
    /// with deep-linkable evidence, use [`Self::execute_with_context`].
    pub fn execute(&self, ctx: &EvalContext) -> VerificationResult {
        let runner = VerificationRunner::new();
        let mut result = runner.verify(self, ctx);

        // Recursively execute sub-cases and merge results
        for sub in &self.sub_cases {
            let sub_result = sub.execute(ctx);
            result.verdict = result.verdict.aggregate(sub_result.verdict);
            result
                .requirement_results
                .extend(sub_result.requirement_results);
            result.diagnostics.extend(sub_result.diagnostics);
        }

        result
    }

    /// Execute this verification case against a runtime session context.
    ///
    /// Functionally identical to [`Self::execute`], but the returned
    /// `VerificationResult` is expected to be lifted to [`Verdict`] via the
    /// configured runner so emitted verdicts carry a populated `evidence`
    /// pointer back to `(session_id, tick)`.
    pub fn execute_with_context(
        &self,
        ctx: &EvalContext,
        verdict_ctx: VerdictContext,
    ) -> VerificationResult {
        let runner = VerificationRunner::with_context(verdict_ctx.clone());
        let mut result = runner.verify(self, ctx);

        // Recursively execute sub-cases under the same session context.
        for sub in &self.sub_cases {
            let sub_result = sub.execute_with_context(ctx, verdict_ctx.clone());
            result.verdict = result.verdict.aggregate(sub_result.verdict);
            result
                .requirement_results
                .extend(sub_result.requirement_results);
            result.diagnostics.extend(sub_result.diagnostics);
        }

        result
    }
}

/// A step within a use case scenario.
///
/// Steps execute sequentially and may produce variable bindings that
/// feed into subsequent steps.
#[derive(Debug, Clone)]
pub struct UseCaseStep {
    /// Step identifier.
    pub id: String,
    /// Human-readable description of this step.
    pub description: String,
    /// The actor performing this step (if any).
    pub actor: Option<String>,
    /// Pre-computed outcome of executing this step.
    pub result: SetupActionResult,
}

/// Compiled use case.
#[derive(Debug, Clone)]
pub struct UseCaseIR {
    /// Case identifier.
    pub id: String,
    /// Case name.
    pub name: String,
    /// Subject of the use case.
    pub subject: Option<String>,
    /// Actors involved.
    pub actors: Vec<String>,
    /// Objective description.
    pub objective: Option<String>,
    /// Ordered scenario steps.
    pub steps: Vec<UseCaseStep>,
    /// Included sub-use-cases (IncludeUseCaseUsage).
    pub includes: Vec<UseCaseIR>,
}

/// Compiled analysis case.
#[derive(Debug, Clone)]
pub struct AnalysisCaseIR {
    /// Case identifier.
    pub id: String,
    /// Case name.
    pub name: String,
    /// Subject of analysis.
    pub subject: Option<String>,
    /// Objective member name, as an opaque string.
    ///
    /// DEPRECATED (FORM-B Tier-1): this is at most the `objective` member's NAME,
    /// not its verdict-bearing semantics. The spec objective is a `RequirementCheck`
    /// whose subject binds to the analysis result (§7.23.2 / `Cases.sysml:40-47`);
    /// that lives in [`Self::objective_requirements`]. Do not add new callers — the
    /// next tier replaces this string field outright (design plan §4.1).
    pub objective: Option<String>,
    /// Objective requirement-checks: the verified requirement(s) of the case's
    /// `objective`, each with the analysis **result** bound to its subject per
    /// §7.23.2 ("the subject of the objective is always bound to the result of the
    /// analysis case"; `Cases.sysml:46` `subject subj default Case::result`, which
    /// `AnalysisCase` does NOT override — unlike a verification case, which binds
    /// the case subject). Empty when the case declares no `verify`'d objective.
    /// Verdicts are produced via [`Self::verify_objective`] through the one engine.
    pub objective_requirements: Vec<RequirementCheck>,
    /// Index-aligned with [`Self::objective_requirements`]: each objective
    /// requirement's subject NAME (the binding key the result binds under). Used by
    /// [`Self::run_and_verify`] to re-bind the subject to a SOLVER-EXECUTED result
    /// that is not known at compile time (§7.23.1 executed result sources). `None`
    /// where a requirement declares no subject.
    pub objective_subject_names: Vec<Option<String>>,
    /// External tool name from ToolExecution metadata.
    pub tool_name: Option<String>,
    /// External tool URI from ToolExecution metadata.
    pub tool_uri: Option<String>,
    /// Input/output parameters for solver invocation.
    pub parameters: Vec<SolverParam>,
    /// Constraints to satisfy during analysis.
    pub constraints: Vec<ConstraintIR>,
    /// Result expression text (e.g. from `return` attribute).
    pub result_expression: Option<String>,
}

/// An analysis objective verified against an ALREADY-solved result.
///
/// Produced by [`AnalysisCaseIR::verify_solved_objective`] so a caller that
/// has already run the solver (e.g. `sysml.analysis.run`, which needs the
/// solver outputs for its own result surface) can obtain the objective verdict
/// WITHOUT a second solve, and project it through the shared verdict surface.
/// The `case` + `context` are exactly what produced `result`, so re-serializing
/// per-requirement detail (actual/expected/margin) resolves against the
/// executed values.
pub struct SolvedObjective {
    /// The objective verdict, through the ONE [`VerificationRunner`].
    pub result: VerificationResult,
    /// The transient verification case the objective was verified as (its
    /// requirements are the case's `objective_requirements`, carrying the
    /// executed-result bindings).
    pub case: VerificationCaseIR,
    /// The context the verdict was evaluated against — case inputs + solver
    /// outputs, with the executed result exposed under each objective subject
    /// name so display re-measurement resolves to the executed value.
    pub context: EvalContext,
}

impl AnalysisCaseIR {
    /// Execute this analysis case using the given solver registry.
    ///
    /// Looks up a solver by `tool_name`. If no solver is registered for the
    /// tool name, falls back to the registry's default solver. If no solver
    /// is available at all, returns `SolverError::Unsupported`.
    pub fn execute(
        &self,
        registry: &SolverRegistry,
        context: &EvalContext,
    ) -> Result<SolverResult, SolverError> {
        let solver = match &self.tool_name {
            Some(name) => registry
                .get(name)
                .or_else(|| registry.default_solver())
                .ok_or_else(|| {
                    SolverError::Unsupported(format!("no solver registered for '{}'", name))
                })?,
            None => registry.default_solver().ok_or_else(|| {
                SolverError::Unsupported("no default solver registered".to_owned())
            })?,
        };
        solver.solve(&self.parameters, &self.constraints, context)
    }

    /// Verify the analysis case's objective and return a verdict.
    ///
    /// §7.23.2: "the subject of the objective is always bound to the result of the
    /// analysis case." Each objective requirement-check already carries that binding
    /// (the result keyed under its subject's name — see [`Self::objective_requirements`]).
    /// The verdict is produced through the ONE verdict engine, [`VerificationRunner`],
    /// by wrapping the objective requirements in a transient verification case — there
    /// is no second verdict path (CLAUDE #4/#5; Inc2b B4 keeps `VerdictKind` authority
    /// in the runner). A case with no `verify`'d objective requirements yields
    /// `Inconclusive` (no basis for a determination — §7.24.1), as does a value-less
    /// result that leaves the objective subject unbound (honest, not masked).
    pub fn verify_objective(&self, ctx: &EvalContext) -> VerificationResult {
        let case = VerificationCaseIR {
            id: self.id.clone(),
            name: self.name.clone(),
            subject: self.subject.clone(),
            setup_actions: Vec::new(),
            requirements: self.objective_requirements.clone(),
            sub_cases: Vec::new(),
            verdict_expression: None,
            bindings: Vec::new(),
        };
        VerificationRunner::new().verify(&case, ctx)
    }

    /// Run the analysis to PRODUCE its result, then verify the objective against it.
    ///
    /// §7.23.1 enumerates the "executed" result sources — analysis actions, an external
    /// solver, ODE integration, simultaneous-equation solve — whose result is only known
    /// AFTER execution (unlike [`Self::verify_objective`]'s compile-time literal /
    /// static-expression sources). This runs the analysis case's OWN solver/ODE via
    /// [`Self::execute`] (NOT the state-machine `verify_with_simulation` path — a separate
    /// command), seeds a context from the solver outputs, resolves the executed result,
    /// binds it to the objective subject (§7.23.2 / `Cases.sysml:46`), and produces the
    /// verdict through the ONE engine ([`VerificationRunner`], B4) — no second verdict path.
    ///
    /// Honest-Inconclusive (§7.24.1): a solver error or non-convergence yields
    /// Inconclusive with a diagnostic, never a verdict on a half-solved state.
    ///
    /// Precedence: the executed result is overlaid AFTER any compile-time static binding
    /// (T1/T2), so `check_requirement`'s in-order, last-writer-wins overlay makes the
    /// EXECUTED value win — by design, not by accident of map ordering.
    pub fn run_and_verify(
        &self,
        registry: &SolverRegistry,
        ctx: &EvalContext,
    ) -> VerificationResult {
        // 1. Execute the analysis's own solver/ODE to PRODUCE the result.
        let solved = match self.execute(registry, ctx) {
            Ok(s) => s,
            Err(e) => {
                return VerificationResult {
                    verdict: VerdictKind::Inconclusive,
                    requirement_results: Vec::new(),
                    diagnostics: vec![Diagnostic::warning(format!(
                        "analysis '{}' could not be executed to produce a result ({}) — \
                         objective verdict is Inconclusive (§7.24.1)",
                        self.name, e
                    ))],
                };
            }
        };
        // 2-6: verdict against the solved result — ONE path, shared with any
        //      caller that already solved (`sysml.analysis.run`), no re-solve.
        self.verify_solved_objective(&solved, ctx).result
    }

    /// Verify the objective against an ALREADY-solved [`SolverResult`] — the
    /// no-re-solve core that [`Self::run_and_verify`] delegates to.
    ///
    /// A caller that has already run the solver (and wants its outputs for a
    /// separate result surface — e.g. `sysml.analysis.run`) passes the solved
    /// result here to obtain the objective verdict without a second solve.
    /// Returns a [`SolvedObjective`] so the verdict can be projected through the
    /// shared verdict surface with per-requirement detail resolving to the
    /// executed values.
    ///
    /// Honest-Inconclusive (§7.24.1): non-convergence yields Inconclusive with a
    /// diagnostic, never a verdict on a half-solved state. (A solver *error* is
    /// caught upstream in [`Self::run_and_verify`], before solving succeeds.)
    ///
    /// Precedence: the executed result is overlaid AFTER any compile-time static
    /// binding (T1/T2), so `check_requirement`'s in-order, last-writer-wins
    /// overlay makes the EXECUTED value win — by design, not by accident.
    pub fn verify_solved_objective(
        &self,
        solved: &SolverResult,
        ctx: &EvalContext,
    ) -> SolvedObjective {
        // 2. Non-convergence → Inconclusive: a determination could not be made (§7.24.1),
        //    not a silent pass/fail on an unconverged state.
        if !solved.converged {
            let case = VerificationCaseIR {
                id: self.id.clone(),
                name: self.name.clone(),
                subject: self.subject.clone(),
                setup_actions: Vec::new(),
                requirements: self.objective_requirements.clone(),
                sub_cases: Vec::new(),
                verdict_expression: None,
                bindings: Vec::new(),
            };
            return SolvedObjective {
                result: VerificationResult {
                    verdict: VerdictKind::Inconclusive,
                    requirement_results: Vec::new(),
                    diagnostics: vec![Diagnostic::warning(format!(
                        "analysis '{}' did not converge — objective verdict is Inconclusive",
                        self.name
                    ))],
                },
                case,
                context: ctx.scratch_snapshot(),
            };
        }

        // 3. Verify-time context: the case inputs (from parameters) + the solver outputs.
        //    The result expression / objective are evaluated against this.
        let mut exec_ctx = ctx.scratch_snapshot();
        for param in &self.parameters {
            if let Some(value) = &param.value {
                exec_ctx.set(param.sysml_name.clone(), value.clone());
            }
        }
        for (name, value) in &solved.outputs {
            exec_ctx.set(name.clone(), value.clone());
        }

        // 4. Resolve the EXECUTED result: the result expression evaluated over the solved
        //    context (`return result = <expr over solved vars>`), else a directly-solved
        //    output named `result` (the spec result feature, Cases.sysml:49). Non-scalar /
        //    unresolvable → None, leaving any compile-time binding in place.
        let executed_result = self
            .result_expression
            .as_deref()
            .and_then(|expr| compile_simple_expression(expr).ok())
            .and_then(|ir| ExpressionEvaluator::new().eval(&ir, &exec_ctx).ok())
            .filter(|v| {
                matches!(
                    v,
                    Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::String(_)
                )
            })
            .or_else(|| match solved.outputs.get("result") {
                Some(v @ (Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::String(_))) => {
                    Some(v.clone())
                }
                _ => None,
            });

        // 5. Overlay the executed result under each objective requirement's subject name
        //    (index-aligned `objective_subject_names`). Pushed last → shadows the
        //    compile-time static binding (the executed result wins).
        let mut requirements = self.objective_requirements.clone();
        if let Some(result) = &executed_result {
            for (req, subject) in requirements
                .iter_mut()
                .zip(self.objective_subject_names.iter())
            {
                if let Some(name) = subject {
                    req.bindings.push((name.clone(), result.clone()));
                }
            }
        }

        // 6. Verdict through the ONE engine over the executed context.
        let case = VerificationCaseIR {
            id: self.id.clone(),
            name: self.name.clone(),
            subject: self.subject.clone(),
            setup_actions: Vec::new(),
            requirements,
            sub_cases: Vec::new(),
            verdict_expression: None,
            bindings: Vec::new(),
        };
        let result = VerificationRunner::new().verify(&case, &exec_ctx);

        // Display context: expose the executed result under each objective
        // subject name so a per-requirement re-measurement (actual/expected)
        // resolves to the executed value. Done AFTER the verdict so it cannot
        // perturb the verdict precedence established via `req.bindings` above.
        let mut display_ctx = exec_ctx;
        if let Some(result_value) = &executed_result {
            for subject in self.objective_subject_names.iter().flatten() {
                display_ctx.set(subject.clone(), result_value.clone());
            }
        }

        SolvedObjective {
            result,
            case,
            context: display_ctx,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::expressions::EvalContext;
    use crate::solver_plugin::SolverParam;
    use crate::solver_registry::SolverRegistry;
    use crate::ConstraintIR;
    use sysml_core::Value;

    #[test]
    fn analysis_case_ir_has_new_fields() {
        let ir = AnalysisCaseIR {
            id: "ac1".into(),
            name: "Thermal".into(),
            subject: Some("engine".into()),
            objective: Some("compute heat".into()),
            objective_requirements: vec![],
            objective_subject_names: vec![],
            tool_name: Some("builtin:propagation".into()),
            tool_uri: Some("http://example.com".into()),
            parameters: vec![SolverParam {
                sysml_name: "temp".into(),
                tool_name: Some("T".into()),
                value: Some(Value::Float(300.0)),
                direction: crate::solver_plugin::ParamDirection::In,
            }],
            constraints: vec![ConstraintIR::new("T < 500")],
            result_expression: Some("T * 2".into()),
        };

        assert_eq!(ir.tool_name.as_deref(), Some("builtin:propagation"));
        assert_eq!(ir.tool_uri.as_deref(), Some("http://example.com"));
        assert_eq!(ir.parameters.len(), 1);
        assert_eq!(ir.constraints.len(), 1);
        assert_eq!(ir.result_expression.as_deref(), Some("T * 2"));
    }

    #[test]
    fn analysis_case_execute_with_default_solver() {
        let ir = AnalysisCaseIR {
            id: "ac1".into(),
            name: "Propagation Test".into(),
            subject: None,
            objective: None,
            objective_requirements: vec![],
            objective_subject_names: vec![],
            tool_name: None, // Use default solver
            tool_uri: None,
            parameters: vec![SolverParam {
                sysml_name: "x".into(),
                tool_name: None,
                value: Some(Value::Float(42.0)),
                direction: crate::solver_plugin::ParamDirection::In,
            }],
            constraints: vec![],
            result_expression: None,
        };

        let registry = SolverRegistry::with_builtins();
        let ctx = EvalContext::new();
        let result = ir.execute(&registry, &ctx);
        assert!(result.is_ok());
        let res = result.unwrap();
        assert!(res.converged);
        assert_eq!(res.outputs.get("x"), Some(&Value::Float(42.0)));
    }

    #[test]
    fn analysis_case_execute_with_named_solver() {
        let ir = AnalysisCaseIR {
            id: "ac2".into(),
            name: "Evaluation Test".into(),
            subject: None,
            objective: None,
            objective_requirements: vec![],
            objective_subject_names: vec![],
            tool_name: Some("builtin:evaluate".into()),
            tool_uri: None,
            parameters: vec![
                SolverParam {
                    sysml_name: "speed".into(),
                    tool_name: None,
                    value: Some(Value::Float(80.0)),
                    direction: crate::solver_plugin::ParamDirection::In,
                },
                SolverParam {
                    sysml_name: "limit".into(),
                    tool_name: None,
                    value: Some(Value::Float(100.0)),
                    direction: crate::solver_plugin::ParamDirection::In,
                },
            ],
            constraints: vec![ConstraintIR::new("speed < limit")],
            result_expression: None,
        };

        let registry = SolverRegistry::with_builtins();
        let ctx = EvalContext::new();
        let result = ir.execute(&registry, &ctx).unwrap();
        assert_eq!(result.outputs.get("all_passed"), Some(&Value::Bool(true)));
    }

    #[test]
    fn analysis_case_execute_falls_back_to_default() {
        // Tool name is specified but not registered; should fall back to default
        let ir = AnalysisCaseIR {
            id: "ac3".into(),
            name: "Fallback Test".into(),
            subject: None,
            objective: None,
            objective_requirements: vec![],
            objective_subject_names: vec![],
            tool_name: Some("nonexistent:tool".into()),
            tool_uri: None,
            parameters: vec![SolverParam {
                sysml_name: "a".into(),
                tool_name: None,
                value: Some(Value::Float(7.0)),
                direction: crate::solver_plugin::ParamDirection::In,
            }],
            constraints: vec![],
            result_expression: None,
        };

        let registry = SolverRegistry::with_builtins();
        let ctx = EvalContext::new();
        let result = ir.execute(&registry, &ctx);
        assert!(result.is_ok(), "should fall back to default solver");
    }

    #[test]
    fn analysis_case_execute_no_solver_errors() {
        let ir = AnalysisCaseIR {
            id: "ac4".into(),
            name: "No Solver".into(),
            subject: None,
            objective: None,
            objective_requirements: vec![],
            objective_subject_names: vec![],
            tool_name: Some("missing".into()),
            tool_uri: None,
            parameters: vec![],
            constraints: vec![],
            result_expression: None,
        };

        // Empty registry: no solvers at all
        let registry = SolverRegistry::new();
        let ctx = EvalContext::new();
        let result = ir.execute(&registry, &ctx);
        assert!(result.is_err());
    }
}
