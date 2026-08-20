//! # Trade Study Framework
//!
//! Evaluates design alternatives against an objective function or constraint
//! set, selecting the best alternative based on a minimize/maximize objective.
//!
//! ## SysML v2 Mapping
//!
//! From the standard library `TradeStudies.sysml`:
//! - `TradeStudy` — analysis case that evaluates alternatives
//! - `EvaluationFunction` — scoring function for each alternative
//! - `MinimizeObjective` / `MaximizeObjective` — optimization direction
//!
//! ## Usage
//!
//! ```text
//! TradeStudyIR::new("material_selection")
//!     .with_alternative(AlternativeIR { name: "steel", parameters: {mass: 100, cost: 500} })
//!     .with_alternative(AlternativeIR { name: "aluminum", parameters: {mass: 60, cost: 800} })
//!     .with_evaluation_expr(expr)   // compiled ExprIR for scoring
//!     .with_objective(TradeStudyObjective::Minimize)
//!     .execute(&base_ctx)
//! ```

#![allow(clippy::indexing_slicing)]
use std::collections::HashMap;

use sysml_core::{ElementKind, ModelGraph, Value};
use sysml_span::Diagnostic;

use crate::expressions::{compile_simple_expression, EvalContext, ExprIR, ExpressionEvaluator};
use crate::ConstraintIR;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single design alternative to evaluate.
#[derive(Debug, Clone)]
pub struct AlternativeIR {
    /// Human-readable name for this alternative.
    pub name: String,
    /// Parameter overrides for this alternative (e.g., {"mass": 10.0, "cost": 500}).
    pub parameters: HashMap<String, Value>,
}

/// Trade study objective direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeStudyObjective {
    /// Select the alternative with the lowest score.
    Minimize,
    /// Select the alternative with the highest score.
    Maximize,
}

/// Trade study IR compiled from SysML AnalysisCaseUsage with trade study pattern.
#[derive(Debug, Clone)]
pub struct TradeStudyIR {
    /// Trade study name.
    pub name: String,
    /// Design alternatives to evaluate.
    pub alternatives: Vec<AlternativeIR>,
    /// Expression to evaluate for each alternative (returns a numeric score).
    /// Used for single-objective studies.
    pub evaluation_expr: Option<ExprIR>,
    /// Named expressions for multi-objective Pareto optimization.
    /// Each entry is `(objective_name, expression)`.
    pub multi_objective_exprs: Vec<(String, ExprIR)>,
    /// Constraints that each alternative must satisfy.
    pub constraints: Vec<ConstraintIR>,
    /// Optimization objective (minimize or maximize).
    pub objective: TradeStudyObjective,
}

/// Result of executing a trade study.
#[derive(Debug, Clone)]
pub struct TradeStudyResult {
    /// Each alternative's name and evaluated score.
    pub scores: Vec<(String, f64)>,
    /// Name of the best alternative.
    pub best: String,
    /// Best score value.
    pub best_score: f64,
}

/// Result of a multi-objective trade study (Pareto optimization).
#[derive(Debug, Clone)]
pub struct ParetoResult {
    /// Name of the study.
    pub study_name: String,
    /// Names of the objectives (one per expression).
    pub objective_names: Vec<String>,
    /// All alternatives with their scores on each objective.
    pub all_scores: Vec<(String, Vec<f64>)>,
    /// Non-dominated alternatives (Pareto front).
    pub pareto_front: Vec<(String, Vec<f64>)>,
}

/// Check if `a` Pareto-dominates `b` (all objectives maximize).
///
/// A dominates B if A >= B on all objectives AND A > B on at least one.
pub(crate) fn pareto_dominates(a: &[f64], b: &[f64]) -> bool {
    let all_ge = a.iter().zip(b.iter()).all(|(ai, bi)| ai >= bi);
    let any_gt = a.iter().zip(b.iter()).any(|(ai, bi)| ai > bi);
    all_ge && any_gt
}

/// Compute the Pareto front from a set of scored alternatives.
///
/// Returns the subset of `scores` where no other alternative dominates it.
pub(crate) fn compute_pareto_front(scores: &[(String, Vec<f64>)]) -> Vec<(String, Vec<f64>)> {
    scores
        .iter()
        .filter(|(_, s)| {
            // Keep if no other alternative dominates this one
            !scores.iter().any(|(_, other)| pareto_dominates(other, s))
        })
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// Builder API
// ---------------------------------------------------------------------------

impl TradeStudyIR {
    /// Create a new trade study with the given name.
    ///
    /// Defaults to `Maximize` objective (higher scores = better), no
    /// alternatives, no evaluation expression, and no constraints.
    pub fn new(name: impl Into<String>) -> Self {
        TradeStudyIR {
            name: name.into(),
            alternatives: Vec::new(),
            evaluation_expr: None,
            multi_objective_exprs: Vec::new(),
            constraints: Vec::new(),
            objective: TradeStudyObjective::Maximize,
        }
    }

    /// Add a design alternative.
    pub fn with_alternative(mut self, alt: AlternativeIR) -> Self {
        self.alternatives.push(alt);
        self
    }

    /// Set the evaluation expression (compiled ExprIR).
    pub fn with_evaluation_expr(mut self, expr: ExprIR) -> Self {
        self.evaluation_expr = Some(expr);
        self
    }

    /// Add a named objective expression for multi-objective Pareto optimization.
    pub fn with_objective_expr(mut self, name: impl Into<String>, expr: ExprIR) -> Self {
        self.multi_objective_exprs.push((name.into(), expr));
        self
    }

    /// Add a constraint that alternatives must satisfy.
    pub fn with_constraint(mut self, c: ConstraintIR) -> Self {
        self.constraints.push(c);
        self
    }

    /// Set the optimization objective.
    pub fn with_objective(mut self, obj: TradeStudyObjective) -> Self {
        self.objective = obj;
        self
    }

    // -----------------------------------------------------------------------
    // Execution
    // -----------------------------------------------------------------------

    /// Execute the trade study: evaluate each alternative and select the best.
    ///
    /// For each alternative the method:
    /// 1. Clones `base_ctx` and overrides parameters from the alternative.
    /// 2. If `evaluation_expr` is set, evaluates it to obtain a numeric score.
    /// 3. If no `evaluation_expr`, scores by counting how many constraints pass.
    /// 4. Selects the best alternative according to `objective`.
    ///
    /// # Errors
    ///
    /// Returns `Err` when:
    /// - No alternatives are provided.
    /// - The evaluation expression produces a non-numeric value.
    /// - Expression compilation or evaluation fails.
    pub fn execute(&self, base_ctx: &EvalContext) -> Result<TradeStudyResult, String> {
        if self.alternatives.is_empty() {
            return Err("trade study has no alternatives to evaluate".into());
        }

        let evaluator = ExpressionEvaluator::new();
        let mut scores: Vec<(String, f64)> = Vec::with_capacity(self.alternatives.len());

        for alt in &self.alternatives {
            // 1. Clone context and apply parameter overrides
            let mut ctx = base_ctx.scratch_snapshot();
            for (k, v) in &alt.parameters {
                ctx.set(k.clone(), v.clone());
            }

            // 2. Compute the score
            let score = if let Some(expr) = &self.evaluation_expr {
                // Evaluate the scoring expression
                let val = evaluator.eval(expr, &ctx).map_err(|e| {
                    format!("evaluation error for alternative '{}': {}", alt.name, e)
                })?;
                value_to_f64(&val).ok_or_else(|| {
                    format!(
                        "evaluation expression for '{}' returned non-numeric value: {:?}",
                        alt.name, val
                    )
                })?
            } else if let Some(score_val) = alt.parameters.get("score") {
                // Fallback: use the "score" attribute from the alternative
                value_to_f64(score_val).unwrap_or(0.0)
            } else {
                // Last resort: score = number of constraints that pass
                self.count_passing_constraints(&ctx, &evaluator)
            };

            scores.push((alt.name.clone(), score));
        }

        // 3. Select the best based on objective
        // Invariant: `scores` is non-empty because `alternatives` is non-empty
        #[allow(clippy::unwrap_used)]
        let best_idx = match self.objective {
            TradeStudyObjective::Minimize => scores
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
                .unwrap(),
            TradeStudyObjective::Maximize => scores
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
                .unwrap(),
        };

        let (best_name, best_score) = scores[best_idx].clone();

        Ok(TradeStudyResult {
            scores,
            best: best_name,
            best_score,
        })
    }

    /// Execute a multi-objective trade study (Pareto optimization).
    ///
    /// Evaluates each alternative against multiple objective expressions and
    /// computes the Pareto front (set of non-dominated alternatives). All
    /// objectives are treated as maximize; to minimize an objective, negate
    /// its expression.
    ///
    /// `expressions` is a list of `(name, ExprIR)` pairs, one per objective.
    ///
    /// # Errors
    ///
    /// Returns `Err` when:
    /// - No alternatives are provided.
    /// - No expressions are provided.
    /// - An expression produces a non-numeric value.
    pub fn execute_multi(
        &self,
        expressions: &[(String, ExprIR)],
        ctx: &EvalContext,
    ) -> Result<ParetoResult, String> {
        if self.alternatives.is_empty() {
            return Err("trade study has no alternatives to evaluate".into());
        }
        if expressions.is_empty() {
            return Err("multi-objective trade study requires at least one expression".into());
        }

        let evaluator = ExpressionEvaluator::new();
        let objective_names: Vec<String> = expressions.iter().map(|(n, _)| n.clone()).collect();
        let mut all_scores: Vec<(String, Vec<f64>)> = Vec::with_capacity(self.alternatives.len());

        for alt in &self.alternatives {
            // Clone context and apply parameter overrides
            let mut alt_ctx = ctx.scratch_snapshot();
            for (k, v) in &alt.parameters {
                alt_ctx.set(k.clone(), v.clone());
            }

            // Evaluate each objective expression
            let mut scores = Vec::with_capacity(expressions.len());
            for (obj_name, expr) in expressions {
                let val = evaluator.eval(expr, &alt_ctx).map_err(|e| {
                    format!(
                        "evaluation error for alternative '{}', objective '{}': {}",
                        alt.name, obj_name, e
                    )
                })?;
                let score = value_to_f64(&val).ok_or_else(|| {
                    format!(
                        "objective '{}' for '{}' returned non-numeric value: {:?}",
                        obj_name, alt.name, val
                    )
                })?;
                scores.push(score);
            }
            all_scores.push((alt.name.clone(), scores));
        }

        let pareto_front = compute_pareto_front(&all_scores);

        Ok(ParetoResult {
            study_name: self.name.clone(),
            objective_names,
            all_scores,
            pareto_front,
        })
    }

    /// Execute a multi-objective trade study using the IR's stored objective
    /// expressions (populated by `with_objective_expr` or `compile_trade_study`).
    ///
    /// This is a convenience wrapper around [`execute_multi`](Self::execute_multi).
    pub fn execute_pareto(&self, ctx: &EvalContext) -> Result<ParetoResult, String> {
        self.execute_multi(&self.multi_objective_exprs, ctx)
    }

    /// Count how many constraints pass for the given context.
    fn count_passing_constraints(&self, ctx: &EvalContext, evaluator: &ExpressionEvaluator) -> f64 {
        let mut pass_count: f64 = 0.0;
        for c in &self.constraints {
            if let Ok(expr_ir) = compile_simple_expression(&c.expr) {
                if let Ok(Value::Bool(true)) = evaluator.eval(&expr_ir, ctx) {
                    pass_count += 1.0;
                }
            }
        }
        pass_count
    }
}

/// Extract an f64 from a Value, if numeric.
fn value_to_f64(val: &Value) -> Option<f64> {
    match val {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f64),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Compiler: ModelGraph → TradeStudyIR
// ---------------------------------------------------------------------------

/// Compile a trade study from a `ModelGraph` by name.
///
/// Looks for an `AnalysisCaseUsage` with the given name that follows the
/// trade study pattern:
/// - Child `PartUsage`, `ItemUsage`, or `AttributeUsage` elements as alternatives
/// - Each alternative has `AttributeUsage` children with parameter overrides
/// - Optional `"objective"` property (`"minimize"` or `"maximize"`)
/// - Optional evaluation expression from `"result"` or `"expression"` properties
/// - Child `ConstraintUsage` elements as constraints
pub fn compile_trade_study(
    name: &str,
    graph: &ModelGraph,
) -> Result<TradeStudyIR, Vec<Diagnostic>> {
    // 1. Find the analysis case element by name
    let case_elem = graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::AnalysisCaseUsage && e.name.as_deref() == Some(name))
        .ok_or_else(|| {
            vec![Diagnostic::error(format!(
                "no analysis case '{}' found",
                name
            ))]
        })?;

    let case_id = case_elem.id.clone();

    // 2. Extract alternatives — child PartUsage/ItemUsage elements
    //    (AttributeUsage children are properties of the study, not alternatives)
    let mut alternatives = Vec::new();
    for child in graph.children_of(&case_id) {
        if !matches!(child.kind, ElementKind::PartUsage | ElementKind::ItemUsage) {
            continue;
        }

        let alt_name = child.name.clone().unwrap_or_else(|| child.id.to_string());
        let mut params = HashMap::new();

        // Extract attribute values from this alternative's children
        for attr in graph.children_of(&child.id) {
            if attr.kind == ElementKind::AttributeUsage {
                if let Some(attr_name) = &attr.name {
                    let val = attr
                        .get_prop("default")
                        .or_else(|| attr.get_prop("value"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    params.insert(attr_name.clone(), val);
                }
            }
        }

        alternatives.push(AlternativeIR {
            name: alt_name,
            parameters: params,
        });
    }

    // 3. Detect objective, evaluation expression(s), and multi-objective expressions
    //    from child AttributeUsage elements.
    //    (in SysML, `attribute objective = "maximize"` becomes a child element, not a property)
    //
    //    For multi-objective Pareto studies, we collect attributes whose names
    //    start with "expression" (e.g., "expression", "expression1", "expression2",
    //    "expression_cost", etc.). If more than one is found, they become the
    //    objectives for `execute_multi`. Single "expression"/"result"/"evaluationFunction"
    //    remains backward-compatible for single-objective studies.
    let mut objective = TradeStudyObjective::Maximize;
    let mut eval_expr = None;
    let mut multi_exprs: Vec<(String, ExprIR)> = Vec::new();

    for child in graph.children_of(&case_id) {
        if child.kind != ElementKind::AttributeUsage {
            continue;
        }
        let Some(attr_name) = child.name.as_deref() else {
            continue;
        };
        match attr_name {
            "objective" | "studyObjective" => {
                let val = child
                    .get_prop("default")
                    .or_else(|| child.get_prop("value"))
                    .and_then(|v| v.as_str());
                if let Some(s) = val {
                    if s.to_lowercase().starts_with("min") {
                        objective = TradeStudyObjective::Minimize;
                    }
                }
            }
            _ if attr_name.starts_with("expression")
                || attr_name == "evaluationFunction"
                || attr_name == "result" =>
            {
                let val = child
                    .get_prop("default")
                    .or_else(|| child.get_prop("value"))
                    .and_then(|v| v.as_str());
                if let Some(s) = val {
                    if let Ok(expr) = compile_simple_expression(s) {
                        // For single-objective backward compat, also set eval_expr
                        // for the canonical names
                        if matches!(attr_name, "expression" | "evaluationFunction" | "result")
                            && eval_expr.is_none()
                        {
                            eval_expr = Some(expr.clone());
                        }
                        multi_exprs.push((attr_name.to_owned(), expr));
                    }
                }
            }
            _ => {}
        }
    }

    // Also check element properties as fallback (for programmatic construction)
    if matches!(objective, TradeStudyObjective::Maximize) {
        if let Some(s) = case_elem.get_prop("objective").and_then(|v| v.as_str()) {
            if s.to_lowercase().starts_with("min") {
                objective = TradeStudyObjective::Minimize;
            }
        }
    }
    if eval_expr.is_none() {
        eval_expr = case_elem
            .get_prop("result")
            .or_else(|| case_elem.get_prop("expression"))
            .and_then(|v| v.as_str())
            .and_then(|s| compile_simple_expression(s).ok());
    }

    // 5. Extract constraints from child ConstraintUsage elements
    let mut constraints = Vec::new();
    for child in graph.children_of(&case_id) {
        if child.kind == ElementKind::ConstraintUsage {
            if let Some(expr_str) = child.get_prop("expression").and_then(|v| v.as_str()) {
                constraints.push(ConstraintIR {
                    expr: expr_str.to_owned(),
                    description: child.name.clone(),
                    owner_id: None,
                    is_negated: false,
                });
            }
        }
    }

    // 6. Validate and build the IR
    if alternatives.is_empty() {
        return Err(vec![Diagnostic::error(format!(
            "trade study '{}' has no alternatives (expected child PartUsage/ItemUsage elements)",
            name
        ))]);
    }

    let mut ir = TradeStudyIR::new(name).with_objective(objective);
    for alt in alternatives {
        ir = ir.with_alternative(alt);
    }
    if let Some(expr) = eval_expr {
        ir = ir.with_evaluation_expr(expr);
    }
    // Populate multi-objective expressions (for Pareto optimization)
    for (obj_name, expr) in multi_exprs {
        ir = ir.with_objective_expr(obj_name, expr);
    }
    for c in constraints {
        ir = ir.with_constraint(c);
    }
    Ok(ir)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::expressions::{compile_simple_expression, BinOp, ExprIR};

    /// Helper: build an AlternativeIR with the given name and parameters.
    fn alt(name: &str, params: Vec<(&str, Value)>) -> AlternativeIR {
        AlternativeIR {
            name: name.into(),
            parameters: params
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }

    // ------------------------------------------------------------------
    // 1. test_trade_study_minimize
    // ------------------------------------------------------------------

    #[test]
    fn test_trade_study_minimize() {
        // 3 alternatives, minimize cost
        let expr = compile_simple_expression("cost").unwrap();
        let study = TradeStudyIR::new("cost_study")
            .with_alternative(alt("steel", vec![("cost", Value::Float(500.0))]))
            .with_alternative(alt("aluminum", vec![("cost", Value::Float(800.0))]))
            .with_alternative(alt("titanium", vec![("cost", Value::Float(1200.0))]))
            .with_evaluation_expr(expr)
            .with_objective(TradeStudyObjective::Minimize);

        let result = study.execute(&EvalContext::new()).unwrap();
        assert_eq!(result.best, "steel");
        assert!((result.best_score - 500.0).abs() < f64::EPSILON);
        assert_eq!(result.scores.len(), 3);
    }

    // ------------------------------------------------------------------
    // 2. test_trade_study_maximize
    // ------------------------------------------------------------------

    #[test]
    fn test_trade_study_maximize() {
        // 3 alternatives, maximize performance
        let expr = compile_simple_expression("performance").unwrap();
        let study = TradeStudyIR::new("perf_study")
            .with_alternative(alt("design_a", vec![("performance", Value::Float(70.0))]))
            .with_alternative(alt("design_b", vec![("performance", Value::Float(95.0))]))
            .with_alternative(alt("design_c", vec![("performance", Value::Float(85.0))]))
            .with_evaluation_expr(expr)
            .with_objective(TradeStudyObjective::Maximize);

        let result = study.execute(&EvalContext::new()).unwrap();
        assert_eq!(result.best, "design_b");
        assert!((result.best_score - 95.0).abs() < f64::EPSILON);
    }

    // ------------------------------------------------------------------
    // 3. test_trade_study_single_alternative
    // ------------------------------------------------------------------

    #[test]
    fn test_trade_study_single_alternative() {
        let expr = compile_simple_expression("weight").unwrap();
        let study = TradeStudyIR::new("single")
            .with_alternative(alt("only_option", vec![("weight", Value::Float(42.0))]))
            .with_evaluation_expr(expr)
            .with_objective(TradeStudyObjective::Minimize);

        let result = study.execute(&EvalContext::new()).unwrap();
        assert_eq!(result.best, "only_option");
        assert!((result.best_score - 42.0).abs() < f64::EPSILON);
        assert_eq!(result.scores.len(), 1);
    }

    // ------------------------------------------------------------------
    // 4. test_trade_study_empty_alternatives
    // ------------------------------------------------------------------

    #[test]
    fn test_trade_study_empty_alternatives() {
        let study = TradeStudyIR::new("empty").with_objective(TradeStudyObjective::Minimize);

        let result = study.execute(&EvalContext::new());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no alternatives"));
    }

    // ------------------------------------------------------------------
    // 5. test_trade_study_with_constraints
    // ------------------------------------------------------------------

    #[test]
    fn test_trade_study_with_constraints() {
        // No evaluation expression — score by constraint satisfaction count.
        // Two constraints: mass < 100 AND cost < 1000
        let study = TradeStudyIR::new("constrained")
            .with_alternative(alt(
                "heavy_cheap",
                vec![("mass", Value::Float(120.0)), ("cost", Value::Float(400.0))],
            ))
            .with_alternative(alt(
                "light_expensive",
                vec![("mass", Value::Float(60.0)), ("cost", Value::Float(1200.0))],
            ))
            .with_alternative(alt(
                "light_cheap",
                vec![("mass", Value::Float(50.0)), ("cost", Value::Float(500.0))],
            ))
            .with_constraint(ConstraintIR::new("mass < 100"))
            .with_constraint(ConstraintIR::new("cost < 1000"))
            .with_objective(TradeStudyObjective::Maximize);

        let result = study.execute(&EvalContext::new()).unwrap();

        // heavy_cheap: mass fails, cost passes -> 1
        // light_expensive: mass passes, cost fails -> 1
        // light_cheap: both pass -> 2
        assert_eq!(result.best, "light_cheap");
        assert!((result.best_score - 2.0).abs() < f64::EPSILON);
    }

    // ------------------------------------------------------------------
    // 6. test_trade_study_builder
    // ------------------------------------------------------------------

    #[test]
    fn test_trade_study_builder() {
        let expr = compile_simple_expression("x + y").unwrap();
        let study = TradeStudyIR::new("builder_test")
            .with_alternative(alt("a", vec![("x", Value::Int(1)), ("y", Value::Int(2))]))
            .with_alternative(alt("b", vec![("x", Value::Int(10)), ("y", Value::Int(20))]))
            .with_evaluation_expr(expr)
            .with_constraint(ConstraintIR::new("x > 0"))
            .with_objective(TradeStudyObjective::Maximize);

        assert_eq!(study.name, "builder_test");
        assert_eq!(study.alternatives.len(), 2);
        assert!(study.evaluation_expr.is_some());
        assert_eq!(study.constraints.len(), 1);
        assert_eq!(study.objective, TradeStudyObjective::Maximize);
    }

    // ------------------------------------------------------------------
    // 7. test_alternative_ir_creation
    // ------------------------------------------------------------------

    #[test]
    fn test_alternative_ir_creation() {
        let alternative = AlternativeIR {
            name: "option_a".into(),
            parameters: {
                let mut m = HashMap::new();
                m.insert("mass".into(), Value::Float(75.0));
                m.insert("cost".into(), Value::Int(600));
                m
            },
        };

        assert_eq!(alternative.name, "option_a");
        assert_eq!(alternative.parameters.len(), 2);
        assert_eq!(
            alternative.parameters.get("mass"),
            Some(&Value::Float(75.0))
        );
        assert_eq!(alternative.parameters.get("cost"), Some(&Value::Int(600)));
    }

    // ------------------------------------------------------------------
    // 8. test_trade_study_result_structure
    // ------------------------------------------------------------------

    #[test]
    fn test_trade_study_result_structure() {
        let expr = compile_simple_expression("score").unwrap();
        let study = TradeStudyIR::new("result_check")
            .with_alternative(alt("alpha", vec![("score", Value::Float(10.0))]))
            .with_alternative(alt("beta", vec![("score", Value::Float(20.0))]))
            .with_alternative(alt("gamma", vec![("score", Value::Float(15.0))]))
            .with_evaluation_expr(expr)
            .with_objective(TradeStudyObjective::Maximize);

        let result = study.execute(&EvalContext::new()).unwrap();

        // Verify scores vec contains all alternatives in order
        assert_eq!(result.scores.len(), 3);
        assert_eq!(result.scores[0].0, "alpha");
        assert!((result.scores[0].1 - 10.0).abs() < f64::EPSILON);
        assert_eq!(result.scores[1].0, "beta");
        assert!((result.scores[1].1 - 20.0).abs() < f64::EPSILON);
        assert_eq!(result.scores[2].0, "gamma");
        assert!((result.scores[2].1 - 15.0).abs() < f64::EPSILON);

        // Verify best selection
        assert_eq!(result.best, "beta");
        assert!((result.best_score - 20.0).abs() < f64::EPSILON);
    }

    // ------------------------------------------------------------------
    // 9. test_trade_study_with_base_context
    // ------------------------------------------------------------------

    #[test]
    fn test_trade_study_with_base_context() {
        // Base context provides a shared variable; alternatives override others.
        let expr = compile_simple_expression("weight * penalty").unwrap();

        let mut base = EvalContext::new();
        base.set("penalty", Value::Float(2.0));

        let study = TradeStudyIR::new("ctx_study")
            .with_alternative(alt("light", vec![("weight", Value::Float(30.0))]))
            .with_alternative(alt("heavy", vec![("weight", Value::Float(80.0))]))
            .with_evaluation_expr(expr)
            .with_objective(TradeStudyObjective::Minimize);

        let result = study.execute(&base).unwrap();
        assert_eq!(result.best, "light");
        assert!((result.best_score - 60.0).abs() < f64::EPSILON);
    }

    // ------------------------------------------------------------------
    // 10. test_trade_study_non_numeric_expr_error
    // ------------------------------------------------------------------

    #[test]
    fn test_trade_study_non_numeric_expr_error() {
        // Evaluation expression returns a boolean, which is non-numeric.
        let expr = compile_simple_expression("x > 0").unwrap();
        let study = TradeStudyIR::new("bad_expr")
            .with_alternative(alt("a", vec![("x", Value::Float(5.0))]))
            .with_evaluation_expr(expr)
            .with_objective(TradeStudyObjective::Minimize);

        let result = study.execute(&EvalContext::new());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("non-numeric"));
    }

    // ------------------------------------------------------------------
    // 11. test_compile_trade_study_from_graph
    // ------------------------------------------------------------------

    #[test]
    fn test_compile_trade_study_from_graph() {
        use sysml_core::ElementId;
        use sysml_core::{Element, ElementKind, ModelGraph};

        let mut graph = ModelGraph::new();

        // Create the analysis case
        let case_id = ElementId::new_v4();
        let mut case_elem = Element::new(case_id.clone(), ElementKind::AnalysisCaseUsage);
        case_elem.name = Some("materialStudy".to_string());
        case_elem.set_prop("objective", Value::String("maximize".to_string()));
        graph.add_element(case_elem);

        // Alternative 1: aluminum
        let alt1_id = ElementId::new_v4();
        let mut alt1 = Element::new(alt1_id.clone(), ElementKind::PartUsage);
        alt1.name = Some("aluminum".to_string());
        alt1.owner = Some(case_id.clone());
        graph.add_element(alt1);

        // Alt1 attribute: strength = 200
        let attr1_id = ElementId::new_v4();
        let mut attr1 = Element::new(attr1_id, ElementKind::AttributeUsage);
        attr1.name = Some("strength".to_string());
        attr1.owner = Some(alt1_id.clone());
        attr1.set_prop("default", Value::Float(200.0));
        graph.add_element(attr1);

        // Alternative 2: steel
        let alt2_id = ElementId::new_v4();
        let mut alt2 = Element::new(alt2_id.clone(), ElementKind::PartUsage);
        alt2.name = Some("steel".to_string());
        alt2.owner = Some(case_id.clone());
        graph.add_element(alt2);

        // Alt2 attribute: strength = 500
        let attr2_id = ElementId::new_v4();
        let mut attr2 = Element::new(attr2_id, ElementKind::AttributeUsage);
        attr2.name = Some("strength".to_string());
        attr2.owner = Some(alt2_id.clone());
        attr2.set_prop("default", Value::Float(500.0));
        graph.add_element(attr2);

        let ir = compile_trade_study("materialStudy", &graph).unwrap();
        assert_eq!(ir.name, "materialStudy");
        assert_eq!(ir.alternatives.len(), 2);
        assert!(matches!(ir.objective, TradeStudyObjective::Maximize));

        // Verify alternatives have the right parameters
        for alt in &ir.alternatives {
            assert!(alt.parameters.contains_key("strength"));
        }
    }

    // ------------------------------------------------------------------
    // 12. test_compile_trade_study_not_found
    // ------------------------------------------------------------------

    #[test]
    fn test_compile_trade_study_not_found() {
        let graph = ModelGraph::new();
        let result = compile_trade_study("nonexistent", &graph);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("no analysis case"));
    }

    // ------------------------------------------------------------------
    // 13. test_compile_trade_study_no_alternatives
    // ------------------------------------------------------------------

    #[test]
    fn test_compile_trade_study_no_alternatives() {
        use sysml_core::ElementId;
        use sysml_core::{Element, ElementKind, ModelGraph};

        let mut graph = ModelGraph::new();

        // Analysis case with no children
        let case_id = ElementId::new_v4();
        let mut case_elem = Element::new(case_id, ElementKind::AnalysisCaseUsage);
        case_elem.name = Some("emptyStudy".to_string());
        graph.add_element(case_elem);

        let result = compile_trade_study("emptyStudy", &graph);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs[0].message.contains("no alternatives"));
    }

    // ------------------------------------------------------------------
    // 14. test_compile_trade_study_minimize_objective
    // ------------------------------------------------------------------

    #[test]
    fn test_compile_trade_study_minimize_objective() {
        use sysml_core::ElementId;
        use sysml_core::{Element, ElementKind, ModelGraph};

        let mut graph = ModelGraph::new();

        let case_id = ElementId::new_v4();
        let mut case_elem = Element::new(case_id.clone(), ElementKind::AnalysisCaseUsage);
        case_elem.name = Some("costStudy".to_string());
        case_elem.set_prop("objective", Value::String("minimize".to_string()));
        graph.add_element(case_elem);

        // One alternative so it doesn't fail on empty
        let alt_id = ElementId::new_v4();
        let mut alt_elem = Element::new(alt_id, ElementKind::ItemUsage);
        alt_elem.name = Some("option_a".to_string());
        alt_elem.owner = Some(case_id);
        graph.add_element(alt_elem);

        let ir = compile_trade_study("costStudy", &graph).unwrap();
        assert!(matches!(ir.objective, TradeStudyObjective::Minimize));
    }

    // ==================================================================
    // Multi-objective Pareto optimization tests (Feature 8.3)
    // ==================================================================

    // ------------------------------------------------------------------
    // 15. test_pareto_dominance
    // ------------------------------------------------------------------

    #[test]
    fn test_pareto_dominance() {
        // Strictly better on all objectives → dominates
        assert!(pareto_dominates(&[3.0, 3.0], &[2.0, 2.0]));

        // Better on one, worse on another → no domination
        assert!(!pareto_dominates(&[3.0, 1.0], &[2.0, 2.0]));

        // Equal on all → no domination (need strictly better on at least one)
        assert!(!pareto_dominates(&[2.0, 2.0], &[2.0, 2.0]));

        // Better on one, equal on the other → dominates
        assert!(pareto_dominates(&[3.0, 2.0], &[2.0, 2.0]));

        // Worse on all → no domination
        assert!(!pareto_dominates(&[1.0, 1.0], &[2.0, 2.0]));

        // Single objective
        assert!(pareto_dominates(&[5.0], &[3.0]));
        assert!(!pareto_dominates(&[3.0], &[5.0]));

        // Three objectives
        assert!(pareto_dominates(&[3.0, 3.0, 3.0], &[2.0, 2.0, 2.0]));
        assert!(!pareto_dominates(&[3.0, 1.0, 3.0], &[2.0, 2.0, 2.0]));
    }

    // ------------------------------------------------------------------
    // 16. test_pareto_front_simple
    // ------------------------------------------------------------------

    #[test]
    fn test_pareto_front_simple() {
        let scores = vec![
            ("A".into(), vec![3.0, 1.0]),
            ("B".into(), vec![1.0, 3.0]),
            ("C".into(), vec![2.0, 2.0]),
            ("D".into(), vec![1.0, 1.0]), // dominated by C (and by A, B)
        ];
        let front = compute_pareto_front(&scores);
        // A, B, C are non-dominated; D is dominated
        assert_eq!(front.len(), 3);
        let names: Vec<&str> = front.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"A"));
        assert!(names.contains(&"B"));
        assert!(names.contains(&"C"));
        assert!(!names.contains(&"D"));
    }

    // ------------------------------------------------------------------
    // 17. test_pareto_front_all_dominated
    // ------------------------------------------------------------------

    #[test]
    fn test_pareto_front_all_dominated() {
        // One alternative dominates all others
        let scores = vec![
            ("best".into(), vec![10.0, 10.0]),
            ("ok".into(), vec![5.0, 5.0]),
            ("bad".into(), vec![1.0, 1.0]),
        ];
        let front = compute_pareto_front(&scores);
        assert_eq!(front.len(), 1);
        assert_eq!(front[0].0, "best");
    }

    // ------------------------------------------------------------------
    // 18. test_pareto_front_none_dominated
    // ------------------------------------------------------------------

    #[test]
    fn test_pareto_front_none_dominated() {
        // All alternatives are on the Pareto front (perfect tradeoffs)
        let scores = vec![("A".into(), vec![5.0, 1.0]), ("B".into(), vec![1.0, 5.0])];
        let front = compute_pareto_front(&scores);
        assert_eq!(front.len(), 2);
    }

    // ------------------------------------------------------------------
    // 19. test_pareto_front_empty
    // ------------------------------------------------------------------

    #[test]
    fn test_pareto_front_empty() {
        let scores: Vec<(String, Vec<f64>)> = vec![];
        let front = compute_pareto_front(&scores);
        assert!(front.is_empty());
    }

    // ------------------------------------------------------------------
    // 20. test_multi_objective_execute
    // ------------------------------------------------------------------

    #[test]
    fn test_multi_objective_execute() {
        // 4 alternatives, 2 objectives: performance and efficiency
        let perf_expr = compile_simple_expression("performance").unwrap();
        let eff_expr = compile_simple_expression("efficiency").unwrap();

        let study = TradeStudyIR::new("multi_study")
            .with_alternative(alt(
                "design_a",
                vec![
                    ("performance", Value::Float(9.0)),
                    ("efficiency", Value::Float(3.0)),
                ],
            ))
            .with_alternative(alt(
                "design_b",
                vec![
                    ("performance", Value::Float(3.0)),
                    ("efficiency", Value::Float(9.0)),
                ],
            ))
            .with_alternative(alt(
                "design_c",
                vec![
                    ("performance", Value::Float(6.0)),
                    ("efficiency", Value::Float(6.0)),
                ],
            ))
            .with_alternative(alt(
                "design_d",
                vec![
                    ("performance", Value::Float(2.0)),
                    ("efficiency", Value::Float(2.0)),
                ],
            ));

        let expressions = vec![
            ("performance".to_string(), perf_expr),
            ("efficiency".to_string(), eff_expr),
        ];

        let result = study
            .execute_multi(&expressions, &EvalContext::new())
            .unwrap();

        assert_eq!(result.study_name, "multi_study");
        assert_eq!(result.objective_names, vec!["performance", "efficiency"]);
        assert_eq!(result.all_scores.len(), 4);

        // design_d (2,2) is dominated by design_c (6,6); others are Pareto-optimal
        assert_eq!(result.pareto_front.len(), 3);
        let front_names: Vec<&str> = result
            .pareto_front
            .iter()
            .map(|(n, _)| n.as_str())
            .collect();
        assert!(front_names.contains(&"design_a"));
        assert!(front_names.contains(&"design_b"));
        assert!(front_names.contains(&"design_c"));
        assert!(!front_names.contains(&"design_d"));
    }

    // ------------------------------------------------------------------
    // 21. test_multi_objective_no_alternatives
    // ------------------------------------------------------------------

    #[test]
    fn test_multi_objective_no_alternatives() {
        let expr = compile_simple_expression("x").unwrap();
        let study = TradeStudyIR::new("empty");

        let result = study.execute_multi(&[("x".to_string(), expr)], &EvalContext::new());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no alternatives"));
    }

    // ------------------------------------------------------------------
    // 22. test_multi_objective_no_expressions
    // ------------------------------------------------------------------

    #[test]
    fn test_multi_objective_no_expressions() {
        let study = TradeStudyIR::new("no_exprs")
            .with_alternative(alt("a", vec![("x", Value::Float(1.0))]));

        let result = study.execute_multi(&[], &EvalContext::new());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("at least one expression"));
    }

    // ------------------------------------------------------------------
    // 23. test_multi_objective_non_numeric_error
    // ------------------------------------------------------------------

    #[test]
    fn test_multi_objective_non_numeric_error() {
        let bool_expr = compile_simple_expression("x > 0").unwrap();
        let study =
            TradeStudyIR::new("bad").with_alternative(alt("a", vec![("x", Value::Float(5.0))]));

        let result = study.execute_multi(&[("check".to_string(), bool_expr)], &EvalContext::new());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("non-numeric"));
    }

    // ------------------------------------------------------------------
    // 24. test_execute_pareto_convenience
    // ------------------------------------------------------------------

    #[test]
    fn test_execute_pareto_convenience() {
        // Test the execute_pareto() convenience method that uses stored exprs
        let perf_expr = compile_simple_expression("perf").unwrap();
        let cost_expr = compile_simple_expression("cost").unwrap();

        let study = TradeStudyIR::new("pareto_conv")
            .with_alternative(alt(
                "cheap_slow",
                vec![("perf", Value::Float(2.0)), ("cost", Value::Float(8.0))],
            ))
            .with_alternative(alt(
                "fast_expensive",
                vec![("perf", Value::Float(8.0)), ("cost", Value::Float(2.0))],
            ))
            .with_objective_expr("perf", perf_expr)
            .with_objective_expr("cost", cost_expr);

        let result = study.execute_pareto(&EvalContext::new()).unwrap();
        assert_eq!(result.pareto_front.len(), 2); // both are non-dominated
        assert_eq!(result.objective_names, vec!["perf", "cost"]);
    }

    // ------------------------------------------------------------------
    // 25. test_multi_objective_with_base_context
    // ------------------------------------------------------------------

    #[test]
    fn test_multi_objective_with_base_context() {
        // Base context provides a multiplier; alternatives override the base values
        let obj1 = compile_simple_expression("score * multiplier").unwrap();
        let obj2 = compile_simple_expression("reliability").unwrap();

        let mut base = EvalContext::new();
        base.set("multiplier", Value::Float(2.0));

        let study = TradeStudyIR::new("ctx_multi")
            .with_alternative(alt(
                "alpha",
                vec![
                    ("score", Value::Float(5.0)),
                    ("reliability", Value::Float(9.0)),
                ],
            ))
            .with_alternative(alt(
                "beta",
                vec![
                    ("score", Value::Float(8.0)),
                    ("reliability", Value::Float(4.0)),
                ],
            ));

        let expressions = vec![
            ("weighted_score".to_string(), obj1),
            ("reliability".to_string(), obj2),
        ];

        let result = study.execute_multi(&expressions, &base).unwrap();

        // alpha: weighted_score = 5*2 = 10, reliability = 9
        // beta:  weighted_score = 8*2 = 16, reliability = 4
        // Neither dominates the other
        assert_eq!(result.all_scores.len(), 2);
        assert!((result.all_scores[0].1[0] - 10.0).abs() < f64::EPSILON);
        assert!((result.all_scores[0].1[1] - 9.0).abs() < f64::EPSILON);
        assert!((result.all_scores[1].1[0] - 16.0).abs() < f64::EPSILON);
        assert!((result.all_scores[1].1[1] - 4.0).abs() < f64::EPSILON);
        assert_eq!(result.pareto_front.len(), 2);
    }

    // ------------------------------------------------------------------
    // 26. test_pareto_front_three_objectives
    // ------------------------------------------------------------------

    #[test]
    fn test_pareto_front_three_objectives() {
        let scores = vec![
            ("A".into(), vec![5.0, 1.0, 3.0]),
            ("B".into(), vec![1.0, 5.0, 3.0]),
            ("C".into(), vec![3.0, 3.0, 5.0]),
            ("D".into(), vec![2.0, 2.0, 2.0]), // dominated by C
            ("E".into(), vec![4.0, 4.0, 4.0]), // dominates D, not dominated by others
        ];
        let front = compute_pareto_front(&scores);
        let names: Vec<&str> = front.iter().map(|(n, _)| n.as_str()).collect();
        // D is dominated by C (and E). All others are non-dominated.
        assert!(!names.contains(&"D"));
        assert!(names.contains(&"A"));
        assert!(names.contains(&"B"));
        assert!(names.contains(&"C"));
        assert!(names.contains(&"E"));
    }

    // ------------------------------------------------------------------
    // 27. test_compile_trade_study_multi_expressions
    // ------------------------------------------------------------------

    #[test]
    fn test_compile_trade_study_multi_expressions() {
        use sysml_core::ElementId;
        use sysml_core::{Element, ElementKind, ModelGraph};

        let mut graph = ModelGraph::new();

        let case_id = ElementId::new_v4();
        let mut case_elem = Element::new(case_id.clone(), ElementKind::AnalysisCaseUsage);
        case_elem.name = Some("multiStudy".to_string());
        graph.add_element(case_elem);

        // Two expression attributes: expression1 and expression2
        let expr1_id = ElementId::new_v4();
        let mut expr1 = Element::new(expr1_id, ElementKind::AttributeUsage);
        expr1.name = Some("expression1".to_string());
        expr1.owner = Some(case_id.clone());
        expr1.set_prop("default", Value::String("strength".to_string()));
        graph.add_element(expr1);

        let expr2_id = ElementId::new_v4();
        let mut expr2 = Element::new(expr2_id, ElementKind::AttributeUsage);
        expr2.name = Some("expression2".to_string());
        expr2.owner = Some(case_id.clone());
        expr2.set_prop("default", Value::String("weight".to_string()));
        graph.add_element(expr2);

        // One alternative
        let alt_id = ElementId::new_v4();
        let mut alt_elem = Element::new(alt_id.clone(), ElementKind::PartUsage);
        alt_elem.name = Some("steel".to_string());
        alt_elem.owner = Some(case_id.clone());
        graph.add_element(alt_elem);

        let s_id = ElementId::new_v4();
        let mut s_attr = Element::new(s_id, ElementKind::AttributeUsage);
        s_attr.name = Some("strength".to_string());
        s_attr.owner = Some(alt_id.clone());
        s_attr.set_prop("default", Value::Float(500.0));
        graph.add_element(s_attr);

        let w_id = ElementId::new_v4();
        let mut w_attr = Element::new(w_id, ElementKind::AttributeUsage);
        w_attr.name = Some("weight".to_string());
        w_attr.owner = Some(alt_id.clone());
        w_attr.set_prop("default", Value::Float(100.0));
        graph.add_element(w_attr);

        let ir = compile_trade_study("multiStudy", &graph).unwrap();
        assert_eq!(ir.multi_objective_exprs.len(), 2);
        let expr_names: Vec<&str> = ir
            .multi_objective_exprs
            .iter()
            .map(|(n, _)| n.as_str())
            .collect();
        assert!(expr_names.contains(&"expression1"));
        assert!(expr_names.contains(&"expression2"));
    }
}
