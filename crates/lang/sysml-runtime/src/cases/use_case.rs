//! Use-case runner, its result types, and the `PassIf` helper.

use crate::expressions::EvalContext;
use sysml_span::Diagnostic;

#[allow(unused_imports)]
use super::*;

// ---------------------------------------------------------------------------
// Use case runner (S2c)
// ---------------------------------------------------------------------------

/// Result of executing a use case.
#[derive(Debug, Clone)]
pub struct UseCaseResult {
    /// Whether execution completed successfully.
    pub success: bool,
    /// The actors that were identified.
    pub actors: Vec<String>,
    /// The subject of the use case.
    pub subject: Option<String>,
    /// Steps executed (in order), with their outcomes.
    pub step_outcomes: Vec<StepOutcome>,
    /// Results from included sub-use-cases.
    pub include_results: Vec<UseCaseResult>,
    /// Diagnostics (errors/warnings).
    pub diagnostics: Vec<Diagnostic>,
}

/// Outcome of a single use case step.
#[derive(Debug, Clone)]
pub struct StepOutcome {
    /// Step identifier.
    pub step_id: String,
    /// Whether this step succeeded.
    pub success: bool,
    /// Message describing the outcome.
    pub message: String,
}

/// Executes a use case by running its steps in order and processing includes.
///
/// The runner:
/// 1. Identifies actors and subject from the use case IR
/// 2. Executes included sub-use-cases first
/// 3. Executes scenario steps in order, threading bindings forward
pub struct UseCaseRunner;

impl UseCaseRunner {
    /// Create a new use case runner.
    pub fn new() -> Self {
        Self
    }

    /// Execute a use case.
    ///
    /// Steps are executed in order. Each successful step's bindings are
    /// accumulated into the context for subsequent steps. If a step fails,
    /// execution continues but the overall result is marked unsuccessful.
    pub fn run(&self, uc: &UseCaseIR, ctx: &EvalContext) -> UseCaseResult {
        let mut enriched_ctx = ctx.scratch_snapshot();
        let mut step_outcomes = Vec::new();
        let mut overall_success = true;
        let mut diagnostics = Vec::new();

        // Execute included sub-use-cases first
        let mut include_results = Vec::new();
        for included in &uc.includes {
            let sub_result = self.run(included, &enriched_ctx);
            if !sub_result.success {
                overall_success = false;
                diagnostics.push(Diagnostic::error(format!(
                    "included use case '{}' failed",
                    included.name,
                )));
            }
            include_results.push(sub_result);
        }

        // Execute steps in order
        for step in &uc.steps {
            match &step.result {
                SetupActionResult::Success { bindings } => {
                    for (name, value) in bindings {
                        enriched_ctx.set(name.clone(), value.clone());
                    }
                    step_outcomes.push(StepOutcome {
                        step_id: step.id.clone(),
                        success: true,
                        message: format!("step '{}' completed", step.description),
                    });
                }
                SetupActionResult::Failure { message } => {
                    overall_success = false;
                    diagnostics.push(Diagnostic::error(format!(
                        "step '{}' failed: {}",
                        step.id, message,
                    )));
                    step_outcomes.push(StepOutcome {
                        step_id: step.id.clone(),
                        success: false,
                        message: message.clone(),
                    });
                }
            }
        }

        UseCaseResult {
            success: overall_success,
            actors: uc.actors.clone(),
            subject: uc.subject.clone(),
            step_outcomes,
            include_results,
            diagnostics,
        }
    }
}

impl Default for UseCaseRunner {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// PassIf calculation (from VerificationCases.sysml)
// ---------------------------------------------------------------------------

/// Implements the `PassIf` calculation from the SysML v2 standard library.
///
/// ```sysml
/// calc def PassIf {
///     in attribute isPassing : Boolean;
///     return attribute verdict : VerdictKind =
///         if isPassing? VerdictKind::pass else VerdictKind::fail;
/// }
/// ```
pub fn pass_if(is_passing: bool) -> VerdictKind {
    if is_passing {
        VerdictKind::Pass
    } else {
        VerdictKind::Fail
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::expressions::EvalContext;
    use sysml_core::Value;

    #[test]
    fn pass_if_true() {
        assert_eq!(pass_if(true), VerdictKind::Pass);
        assert_eq!(pass_if(false), VerdictKind::Fail);
    }

    #[test]
    fn use_case_identifies_actors() {
        let uc = UseCaseIR {
            id: "uc1".into(),
            name: "Drive Vehicle".into(),
            subject: Some("vehicle".into()),
            actors: vec!["driver".into(), "passenger".into()],
            objective: Some("Transport people".into()),
            steps: vec![],
            includes: vec![],
        };

        let ctx = EvalContext::new();
        let runner = UseCaseRunner::new();
        let result = runner.run(&uc, &ctx);
        assert!(result.success);
        assert_eq!(result.actors, vec!["driver", "passenger"]);
        assert_eq!(result.subject.as_deref(), Some("vehicle"));
    }

    #[test]
    fn use_case_include_executes_sub() {
        let sub_uc = UseCaseIR {
            id: "uc-sub".into(),
            name: "Start Engine".into(),
            subject: None,
            actors: vec!["driver".into()],
            objective: None,
            steps: vec![UseCaseStep {
                id: "step-start".into(),
                description: "Turn ignition".into(),
                actor: Some("driver".into()),
                result: SetupActionResult::Success {
                    bindings: vec![("engine_running".into(), Value::Bool(true))],
                },
            }],
            includes: vec![],
        };

        let uc = UseCaseIR {
            id: "uc-main".into(),
            name: "Drive Vehicle".into(),
            subject: Some("vehicle".into()),
            actors: vec!["driver".into()],
            objective: None,
            steps: vec![UseCaseStep {
                id: "step-drive".into(),
                description: "Accelerate".into(),
                actor: Some("driver".into()),
                result: SetupActionResult::Success {
                    bindings: vec![("speed".into(), Value::Int(60))],
                },
            }],
            includes: vec![sub_uc],
        };

        let ctx = EvalContext::new();
        let runner = UseCaseRunner::new();
        let result = runner.run(&uc, &ctx);
        assert!(result.success);
        // The included sub-use-case should have run successfully
        assert_eq!(result.include_results.len(), 1);
        assert!(result.include_results[0].success);
        assert_eq!(result.include_results[0].step_outcomes.len(), 1);
        assert!(result.include_results[0].step_outcomes[0].success);
        // The main steps also executed
        assert_eq!(result.step_outcomes.len(), 1);
        assert!(result.step_outcomes[0].success);
    }

    #[test]
    fn use_case_scenario_steps_in_order() {
        let uc = UseCaseIR {
            id: "uc-seq".into(),
            name: "Sequential Steps".into(),
            subject: None,
            actors: vec!["actor".into()],
            objective: None,
            steps: vec![
                UseCaseStep {
                    id: "step-1".into(),
                    description: "First step".into(),
                    actor: Some("actor".into()),
                    result: SetupActionResult::Success {
                        bindings: vec![("a".into(), Value::Int(1))],
                    },
                },
                UseCaseStep {
                    id: "step-2".into(),
                    description: "Second step".into(),
                    actor: Some("actor".into()),
                    result: SetupActionResult::Success {
                        bindings: vec![("b".into(), Value::Int(2))],
                    },
                },
                UseCaseStep {
                    id: "step-3".into(),
                    description: "Third step".into(),
                    actor: Some("actor".into()),
                    result: SetupActionResult::Success {
                        bindings: vec![("c".into(), Value::Int(3))],
                    },
                },
            ],
            includes: vec![],
        };

        let ctx = EvalContext::new();
        let runner = UseCaseRunner::new();
        let result = runner.run(&uc, &ctx);
        assert!(result.success);
        assert_eq!(result.step_outcomes.len(), 3);
        // Verify steps executed in order
        assert_eq!(result.step_outcomes[0].step_id, "step-1");
        assert_eq!(result.step_outcomes[1].step_id, "step-2");
        assert_eq!(result.step_outcomes[2].step_id, "step-3");
        assert!(result.step_outcomes.iter().all(|s| s.success));
    }
}
