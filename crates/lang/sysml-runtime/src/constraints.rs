//! # sysml-run-constraints
//!
//! Constraint compilation and evaluation for SysML v2.
//!
//! This crate provides:
//! - Extraction of constraints from ModelGraph
//! - Evaluation of constraints using the [`ExpressionEvaluator`] from `sysml-run-expressions`
//!
//! ## Architecture
//!
//! ```text
//! ModelGraph -> extract_constraints() -> ConstraintSet
//!                                            |
//!              evaluate_all(constraints, ctx) -> [EvaluationResult]
//!                        |
//!          ExpressionEvaluator (from sysml-run-expressions)
//! ```
//!
//! ## Spec References
//!
//! - `SysML.xtext:1976-2012` — Constraint grammar
//! - `library.systems/Constraints.sysml` — Standard constraint library
//! - `KerMLExpressions.xtext` — Expression operators

#![allow(clippy::indexing_slicing)]
use std::collections::HashMap;

use crate::expressions::{
    compile_expression, compile_simple_expression, EvaluationError, ExprIR, ExpressionEvaluator,
};
use crate::cases::VerdictKind;
use crate::ConstraintIR;
use sysml_core::{Element, ElementKind, ModelGraph, Value};
use sysml_span::{Diagnostic, Span};

// Re-export EvalContext so consumers of this crate don't need to
// depend on sysml-run-expressions directly.
pub use crate::expressions::EvalContext;

/// A compiled set of constraints.
#[derive(Debug, Clone)]
pub struct ConstraintSet {
    /// The constraints in this set.
    pub constraints: Vec<ConstraintIR>,
}

impl ConstraintSet {
    /// Create a new empty constraint set.
    pub fn new() -> Self {
        ConstraintSet {
            constraints: Vec::new(),
        }
    }

    /// Add a constraint.
    pub fn add(&mut self, constraint: ConstraintIR) {
        self.constraints.push(constraint);
    }

    /// Get the number of constraints.
    pub fn len(&self) -> usize {
        self.constraints.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.constraints.is_empty()
    }
}

impl Default for ConstraintSet {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract constraints from a ModelGraph, filtering elements with a predicate.
///
/// Only elements for which `predicate` returns `true` are considered.
/// This is useful for excluding library/stdlib elements from constraint extraction.
pub fn extract_constraints_filtered(
    graph: &ModelGraph,
    predicate: impl Fn(&Element) -> bool,
) -> ConstraintSet {
    let mut set = ConstraintSet::new();

    // Use kind_index for O(1) lookup instead of scanning all elements.
    let constraint_kinds = [
        ElementKind::ConstraintUsage,
        ElementKind::AssertConstraintUsage,
        ElementKind::ConstraintDefinition,
    ];
    for kind in &constraint_kinds {
        for element in graph.elements_by_kind(kind) {
            if predicate(element) {
                extract_from_element(element, graph, &mut set);
            }
        }
    }

    set
}

/// Extract constraints from a ModelGraph.
///
/// Prefers structured expression subtree children (parser-emitted,
/// pretty-printed for display); falls back to the legacy `constraint` /
/// `expr` / `unresolved_value` string properties for hand-crafted test
/// graphs and runtime-synthesized constraints.
pub fn extract_constraints(graph: &ModelGraph) -> ConstraintSet {
    #[cfg(feature = "tracing")]
    tracing::trace!(
        element_count = graph.element_count(),
        "extracting constraints from model graph"
    );

    let mut set = ConstraintSet::new();

    let constraint_kinds = [
        ElementKind::ConstraintUsage,
        ElementKind::AssertConstraintUsage,
        ElementKind::ConstraintDefinition,
    ];
    for kind in &constraint_kinds {
        for element in graph.elements_by_kind(kind) {
            extract_from_element(element, graph, &mut set);
        }
    }

    #[cfg(feature = "tracing")]
    tracing::debug!(
        constraint_count = set.len(),
        "constraint extraction complete"
    );

    set
}

/// Extract constraint expressions from a single element.
///
/// Post-Phase-6D: AST-first. Parser-produced graphs carry a structured
/// expression subtree; `pretty_print_owner` produces the ConstraintIR
/// display string. Legacy `constraint` / `expr` string props are checked
/// only as a fallback for hand-crafted test graphs that have no AST children.
fn extract_from_element(element: &Element, graph: &ModelGraph, set: &mut ConstraintSet) {
    let owner = element.owner.clone();
    let name = element.name.clone().unwrap_or_default();

    let text = sysml_core::expression_pretty::pretty_print_owner(element, graph)
        .or_else(|| {
            element
                .get_prop("constraint")
                .and_then(|v| v.as_str().map(str::to_owned))
        })
        .or_else(|| {
            element
                .get_prop("expr")
                .and_then(|v| v.as_str().map(str::to_owned))
        });

    if let Some(text) = text {
        let mut constraint = ConstraintIR::new(text).with_description(name);
        constraint.owner_id = owner;
        // Carry `assert not constraint` negation (SysML §7.20) so the eval
        // layer inverts the verdict; parity with the precompile path.
        constraint.is_negated = element
            .get_prop("isNegated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        set.add(constraint);
    }
}

/// Result of extracting and pre-compiling constraints.
///
/// Constraints are compiled eagerly during extraction so that:
/// 1. Syntax errors are caught early (before runtime evaluation)
/// 2. Repeated evaluations avoid re-parsing the expression string
/// 3. Diagnostics can be surfaced to the user at elaboration time
#[derive(Debug, Clone)]
pub struct PrecompiledConstraintSet {
    /// Successfully compiled constraints.
    pub compiled: Vec<TypedConstraint>,
    /// Constraints that failed to compile, with their diagnostics.
    pub failed: Vec<(ConstraintIR, Vec<Diagnostic>)>,
}

impl PrecompiledConstraintSet {
    /// Number of successfully compiled constraints.
    pub fn compiled_count(&self) -> usize {
        self.compiled.len()
    }

    /// Number of constraints that failed compilation.
    pub fn failed_count(&self) -> usize {
        self.failed.len()
    }

    /// Total constraints attempted.
    pub fn total(&self) -> usize {
        self.compiled.len() + self.failed.len()
    }

    /// All diagnostics from failed compilations.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.failed
            .iter()
            .flat_map(|(_, diags)| diags.iter().cloned())
            .collect()
    }

    /// Evaluate all compiled constraints against the given context.
    pub fn evaluate_all(&self, context: &EvalContext) -> Vec<EvaluationResult> {
        self.compiled
            .iter()
            .map(|tc| tc.evaluate(context))
            .collect()
    }
}

/// Extract constraints from a ModelGraph and pre-compile them.
///
/// This is the preferred extraction path for production use. It walks
/// constraint elements directly and compiles each via [`compile_expression`]
/// (AST-first, string-fallback), avoiding the legacy parse→pretty-print→
/// re-parse round-trip through `ConstraintIR.expr`. Elements for which
/// AST compilation fails and that carry no `constraint`/`expr` string prop
/// land in `failed` with diagnostics.
///
/// # Membership
///
/// Only **usages** authored in the subject model are checkable, so the walk
/// is scoped twice:
///
/// 1. **Usages, never definitions.** A `ConstraintDefinition` is a *type*, not
///    an assertion: per the SysML v2 vocabulary it "defines a constraint that
///    **may be asserted** to hold on a system or part of a system"
///    (`SysML-vocab.ttl:247`), and as a `Predicate` it is a kind of Behavior
///    whose `in` parameters are formal and unbound until it is invoked
///    (KerML §8.4.4.8.1). Evaluating a bare `constraint def` against the run
///    context reads its unbound formals and can only ever produce noise —
///    which is exactly what the stdlib's `validOriginDimensions`,
///    `orderSum`, `disjointCauseEffect` &c. were doing. Checking happens
///    through the [`ConstraintUsage`](ElementKind::ConstraintUsage) /
///    [`AssertConstraintUsage`](ElementKind::AssertConstraintUsage) that
///    *uses* the definition and binds its features.
///
/// 2. **The subject model, never the libraries it imports.** `Import` is a
///    name-visibility relationship — it "determines a set of Memberships that
///    become importedMemberships of the importOwningNamespace"
///    (`Kerml-Vocab.ttl:231`) — and says nothing about instantiation or
///    obligation. Importing `Geometry` or `MeasurementReferences` does not
///    make their internal invariants the importing model's obligations, so
///    library elements are skipped via
///    [`ModelGraph::is_library_element`]. This matches the filter
///    `sysml.constraint.check` and `evaluate_constraints_impl` already apply.
pub fn extract_and_precompile(graph: &ModelGraph) -> PrecompiledConstraintSet {
    let mut compiled = Vec::new();
    let mut failed = Vec::new();

    let constraint_kinds = [
        ElementKind::ConstraintUsage,
        ElementKind::AssertConstraintUsage,
    ];

    for kind in &constraint_kinds {
        for element in graph.elements_by_kind(kind) {
            // Libraries are imported for visibility, not adopted as
            // obligations (Kerml-Vocab.ttl:231). Their invariants belong to
            // their own unbound formals, not to this model's run.
            if graph.is_library_element(&element.id) {
                continue;
            }
            // AST-first (Phase 6D): prefer compile_expression which walks
            // the structured expression subtree. Falls back to legacy
            // string props only for hand-crafted test graphs.
            let name = element.name.clone().unwrap_or_default();
            let owner = element.owner.clone();

            match TypedConstraint::from_element(element, graph) {
                Ok(tc) => compiled.push(tc),
                Err(diags) => {
                    // Only surface a failure if the element actually owns
                    // expression content; otherwise (empty constraint
                    // body) stay silent.
                    let has_content =
                        sysml_core::expression_pretty::pretty_print_owner(element, graph).is_some()
                            || element
                                .get_prop("constraint")
                                .and_then(|v| v.as_str())
                                .is_some()
                            || element.get_prop("expr").and_then(|v| v.as_str()).is_some();
                    if has_content {
                        let display =
                            sysml_core::expression_pretty::pretty_print_owner(element, graph)
                                .unwrap_or_else(|| format!("<constraint {}>", name));
                        let mut ir = ConstraintIR::new(display).with_description(name);
                        ir.owner_id = owner;
                        failed.push((ir, diags));
                    }
                }
            }
        }
    }

    #[cfg(feature = "tracing")]
    tracing::info!(
        compiled = compiled.len(),
        failed = failed.len(),
        "constraint extract-and-precompile complete (AST-first)"
    );

    PrecompiledConstraintSet { compiled, failed }
}

/// Pre-compile an existing [`ConstraintSet`] into a [`PrecompiledConstraintSet`].
pub fn precompile_constraint_set(set: &ConstraintSet) -> PrecompiledConstraintSet {
    let mut compiled = Vec::new();
    let mut failed = Vec::new();

    for constraint in &set.constraints {
        match TypedConstraint::compile(constraint.clone()) {
            Ok(typed) => compiled.push(typed),
            Err(diags) => failed.push((constraint.clone(), diags)),
        }
    }

    #[cfg(feature = "tracing")]
    tracing::info!(
        total = set.len(),
        compiled = compiled.len(),
        failed = failed.len(),
        "constraint precompilation complete"
    );

    PrecompiledConstraintSet { compiled, failed }
}

/// The result of evaluating a constraint.
#[derive(Debug, Clone)]
pub struct EvaluationResult {
    /// The constraint that was evaluated.
    pub constraint: ConstraintIR,
    /// Whether the constraint is satisfied.
    ///
    /// When `inconclusive` is true, this field is `false` but does not mean
    /// the constraint was evaluated and failed — it means evaluation could
    /// not complete (e.g. undefined variable).
    pub satisfied: bool,
    /// True when the constraint could not be fully evaluated (e.g. undefined
    /// variable, compilation failure). Consumers should treat this as
    /// "unknown" rather than "failed".
    pub inconclusive: bool,
    /// Any diagnostics from evaluation.
    pub diagnostics: Vec<Diagnostic>,
    /// Live values for each identifier referenced by the constraint
    /// expression, captured at eval time. Only `f64`-coercible values
    /// land here (Bool → 0/1, Int/Float/Quantity/Complex.re → as-is);
    /// identifiers bound to non-scalar values or missing from the
    /// context are omitted. Lets the UI show the user *why* a
    /// constraint passed or failed on this tick without re-reading
    /// the scalar_vars map for each identifier (GAP-CONSTR-002).
    pub operands: HashMap<String, f64>,
}

impl EvaluationResult {
    /// The spec-taxonomy verdict for this evaluation.
    ///
    /// This is the **one home** for the `(satisfied, inconclusive)` →
    /// [`VerdictKind`] translation; every consumer that needs a verdict
    /// (session snapshots, `sysml.constraint.check`, overlays) routes
    /// through here rather than re-deriving it inline.
    ///
    /// An evaluation that could not complete — an unbound parameter, a
    /// non-boolean result, an evaluator error — is
    /// [`Inconclusive`](VerdictKind::Inconclusive), never
    /// [`Fail`](VerdictKind::Fail). `Fail` is a claim that the model *was*
    /// checked and violated the constraint; KerML has no three-valued
    /// Boolean and no coerce-to-false rule for a missing binding (its model
    /// of "no result" is the empty result of a `NullExpression`,
    /// KerML §8.4.4.9.1), so reporting `false` for an unevaluated
    /// constraint asserts something the run never established.
    pub fn verdict(&self) -> VerdictKind {
        if self.inconclusive {
            VerdictKind::Inconclusive
        } else if self.satisfied {
            VerdictKind::Pass
        } else {
            VerdictKind::Fail
        }
    }
}

/// A constraint paired with a pre-compiled expression.
///
/// This avoids re-compiling the expression string on every evaluation.
/// Use [`TypedConstraint::compile`] to create one from a [`ConstraintIR`],
/// or [`TypedConstraint::from_expr`] to wrap an already-compiled [`ExprIR`].
#[derive(Debug, Clone)]
pub struct TypedConstraint {
    /// The underlying constraint IR.
    pub constraint: ConstraintIR,
    /// The pre-compiled expression.
    pub expr_ir: ExprIR,
}

impl TypedConstraint {
    /// Compile a [`ConstraintIR`] into a typed constraint.
    ///
    /// Returns compilation diagnostics on failure.
    pub fn compile(constraint: ConstraintIR) -> Result<Self, Vec<Diagnostic>> {
        let expr_ir = compile_simple_expression(&constraint.expr)?;
        Ok(TypedConstraint {
            constraint,
            expr_ir,
        })
    }

    /// Compile an element's constraint expression AST-first.
    ///
    /// Uses [`compile_expression`] to walk the element's expression
    /// subtree (preferred) or fall back to the legacy `expr`/`constraint`
    /// string props. Display text on the returned `ConstraintIR` is
    /// derived from the AST via the canonical pretty-printer; if no AST
    /// subtree is present the display text comes from the string prop.
    pub fn from_element(element: &Element, graph: &ModelGraph) -> Result<Self, Vec<Diagnostic>> {
        let expr_ir = compile_expression(element, graph)?;

        let display = sysml_core::expression_pretty::pretty_print_owner(element, graph)
            .or_else(|| {
                element
                    .get_prop("constraint")
                    .and_then(|v| v.as_str().map(ToOwned::to_owned))
            })
            .or_else(|| {
                element
                    .get_prop("expr")
                    .and_then(|v| v.as_str().map(ToOwned::to_owned))
            })
            .unwrap_or_else(|| format!("{:?}", expr_ir));

        let mut ir =
            ConstraintIR::new(display).with_description(element.name.clone().unwrap_or_default());
        ir.owner_id = element.owner.clone();
        // `AssertConstraintUsage` with `assert not` carries `isNegated=true`
        // (normalized to bool by elaboration). The verdict inverts at eval time.
        ir.is_negated = element
            .get_prop("isNegated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        Ok(TypedConstraint {
            constraint: ir,
            expr_ir,
        })
    }

    /// Create a typed constraint from a pre-compiled [`ExprIR`].
    pub fn from_expr(expr_ir: ExprIR, description: impl Into<Option<String>>) -> Self {
        let desc = description.into();
        let constraint = ConstraintIR::new(format!("{:?}", expr_ir));
        let constraint = if let Some(d) = desc {
            constraint.with_description(d)
        } else {
            constraint
        };
        TypedConstraint {
            constraint,
            expr_ir,
        }
    }

    /// Evaluate this typed constraint against the given context.
    pub fn evaluate(&self, context: &EvalContext) -> EvaluationResult {
        evaluate_expr(&self.expr_ir, &self.constraint, context)
    }
}

/// Evaluate a pre-compiled [`ExprIR`] as a constraint.
///
/// Skips the compilation step, useful when the expression has already
/// been compiled or was constructed programmatically.
pub fn evaluate_expr(
    expr: &ExprIR,
    constraint: &ConstraintIR,
    context: &EvalContext,
) -> EvaluationResult {
    #[cfg(feature = "tracing")]
    tracing::trace!(
        description = constraint.description.as_deref().unwrap_or(""),
        expr_len = constraint.expr.len(),
        binding_count = context.variables.len(),
        "evaluating precompiled constraint expression"
    );

    let evaluator = ExpressionEvaluator::new();
    let operands = collect_operand_values(expr, context);

    match evaluator.eval(expr, context) {
        Ok(Value::Bool(b)) => EvaluationResult {
            constraint: constraint.clone(),
            // SysML §7.20: a negated `assert not constraint {C}` asserts C is
            // false, so the decided verdict inverts the inner boolean. Only a
            // decided (non-inconclusive) result is inverted — an undecidable
            // inner constraint stays inconclusive (handled in the Err arms).
            satisfied: if constraint.is_negated { !b } else { b },
            inconclusive: false,
            diagnostics: Vec::new(),
            operands,
        },
        Ok(other) => {
            #[cfg(feature = "tracing")]
            tracing::debug!(
                expr = constraint.expr.as_str(),
                result = ?other,
                "constraint evaluated to non-boolean value"
            );
            EvaluationResult {
                constraint: constraint.clone(),
                satisfied: false,
                inconclusive: true,
                diagnostics: vec![Diagnostic::error(format!(
                    "constraint `{}` evaluated to {:?}, expected boolean",
                    constraint.expr, other
                ))],
                operands,
            }
        }
        Err(EvaluationError::UndefinedVariable(ref var)) => {
            #[cfg(feature = "tracing")]
            tracing::debug!(
                expr = constraint.expr.as_str(),
                variable = var.as_str(),
                "constraint has undefined variable — result is inconclusive"
            );
            EvaluationResult {
                constraint: constraint.clone(),
                satisfied: false,
                inconclusive: true,
                diagnostics: vec![Diagnostic::warning(format!(
                    "constraint `{}` references undefined variable `{}` — result is inconclusive",
                    constraint.expr, var
                ))],
                operands,
            }
        }
        Err(e) => {
            #[cfg(feature = "tracing")]
            tracing::debug!(
                expr = constraint.expr.as_str(),
                error = %e,
                "constraint evaluation error"
            );
            EvaluationResult {
                constraint: constraint.clone(),
                satisfied: false,
                inconclusive: true,
                diagnostics: vec![Diagnostic::error(format!(
                    "constraint `{}` evaluation error: {}",
                    constraint.expr, e
                ))],
                operands,
            }
        }
    }
}

/// Walk the expression for its free variables and snapshot each one's
/// current numeric value from the context. Non-scalar values (lists,
/// strings, enums, refs, null) are omitted — the UI can only render
/// operand badges for things with a single numeric reading.
fn collect_operand_values(expr: &ExprIR, context: &EvalContext) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    for name in expr.free_variables() {
        if let Some(v) = context.variables.get(&name) {
            if let Some(f) = value_to_scalar(v) {
                out.insert(name, f);
            }
        }
    }
    out
}

fn value_to_scalar(value: &Value) -> Option<f64> {
    match value {
        Value::Int(v) => Some(*v as f64),
        Value::Float(v) => Some(*v),
        Value::Bool(v) => Some(if *v { 1.0 } else { 0.0 }),
        // Dimensional quantities and complex numbers carry a scalar magnitude;
        // dropping them silently skipped threshold constraints on physics-typed
        // features (matches sysml-runtime snapshot_view::value_to_scalar).
        Value::Quantity { value, .. } => Some(*value),
        Value::Complex { re, .. } => Some(*re),
        _ => None,
    }
}

/// Evaluate a single constraint.
///
/// The constraint expression string is compiled to an [`ExprIR`] via
/// [`compile_simple_expression`] and then evaluated with the
/// [`ExpressionEvaluator`] against the provided context.
pub fn evaluate(constraint: &ConstraintIR, context: &EvalContext) -> EvaluationResult {
    #[cfg(feature = "tracing")]
    tracing::trace!(
        description = constraint.description.as_deref().unwrap_or(""),
        expr_len = constraint.expr.len(),
        binding_count = context.variables.len(),
        "evaluating constraint"
    );

    // Compile the expression string to ExprIR
    let expr_ir = match compile_simple_expression(&constraint.expr) {
        Ok(ir) => ir,
        Err(diags) => {
            #[cfg(feature = "tracing")]
            tracing::debug!(
                expr = constraint.expr.as_str(),
                diagnostic_count = diags.len(),
                "constraint compilation failed"
            );
            return EvaluationResult {
                constraint: constraint.clone(),
                satisfied: false,
                inconclusive: true,
                diagnostics: diags,
                operands: HashMap::new(),
            };
        }
    };

    // Delegate to evaluate_expr for the actual evaluation
    evaluate_expr(&expr_ir, constraint, context)
}

/// Evaluate all constraints in a set.
pub fn evaluate_all(constraints: &ConstraintSet, context: &EvalContext) -> Vec<EvaluationResult> {
    #[cfg(feature = "tracing")]
    tracing::trace!(
        constraint_count = constraints.constraints.len(),
        binding_count = context.variables.len(),
        "evaluating all constraints"
    );

    constraints
        .constraints
        .iter()
        .map(|c| evaluate(c, context))
        .collect()
}

/// Check if all constraints are satisfied.
pub fn all_satisfied(results: &[EvaluationResult]) -> bool {
    results.iter().all(|r| r.satisfied)
}

/// Get failed constraints (excluding inconclusive).
pub fn failed_constraints(results: &[EvaluationResult]) -> Vec<&EvaluationResult> {
    results
        .iter()
        .filter(|r| !r.satisfied && !r.inconclusive)
        .collect()
}

/// Get inconclusive constraints (could not be evaluated).
pub fn inconclusive_constraints(results: &[EvaluationResult]) -> Vec<&EvaluationResult> {
    results.iter().filter(|r| r.inconclusive).collect()
}

// ---------------------------------------------------------------------------
// Requirement satisfaction (S2b)
// ---------------------------------------------------------------------------

/// A compiled requirement pairing assumptions with constraints.
///
/// Follows the SysML v2 requirement pattern: if all assumptions hold, then
/// all constraints must hold.  When any assumption is false the requirement
/// is *vacuously satisfied* (false premise implies anything).
///
/// ## Spec References
///
/// - `library.systems/Requirements.sysml` — RequirementCheck, assumptions/constraints
/// - `SysML-vocab.ttl` — SatisfyRequirementUsage
#[derive(Debug, Clone)]
pub struct RequirementConstraintIR {
    /// Human-readable name for this requirement.
    pub name: String,
    /// Assumption constraints — all must hold for the requirement to be checked.
    pub assumptions: Vec<ConstraintIR>,
    /// Required constraints — all must hold when assumptions are met.
    pub constraints: Vec<ConstraintIR>,
    /// When true the overall satisfaction result is inverted.
    pub is_negated: bool,
}

impl RequirementConstraintIR {
    /// Create a new requirement with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        RequirementConstraintIR {
            name: name.into(),
            assumptions: Vec::new(),
            constraints: Vec::new(),
            is_negated: false,
        }
    }

    /// Add an assumption constraint.
    pub fn with_assumption(mut self, constraint: ConstraintIR) -> Self {
        self.assumptions.push(constraint);
        self
    }

    /// Add a required constraint.
    pub fn with_constraint(mut self, constraint: ConstraintIR) -> Self {
        self.constraints.push(constraint);
        self
    }

    /// Set the negation flag.
    pub fn negated(mut self) -> Self {
        self.is_negated = true;
        self
    }
}

/// Evaluate a requirement against the given context.
///
/// Evaluation follows the SysML v2 requirement satisfaction rules:
///
/// 1. Evaluate all assumptions.
/// 2. If any assumption is false the requirement is **vacuously satisfied**.
/// 3. If all assumptions hold, evaluate all constraints — satisfied only when
///    every constraint is satisfied.
/// 4. If `is_negated` is true, invert the final result.
pub fn evaluate_requirement(
    req: &RequirementConstraintIR,
    context: &EvalContext,
) -> EvaluationResult {
    let mut all_diags = Vec::new();
    let mut any_inconclusive = false;

    // Step 1-2: evaluate assumptions
    let mut all_assumptions_met = true;
    for assumption in &req.assumptions {
        let result = evaluate(assumption, context);
        all_diags.extend(result.diagnostics);
        if result.inconclusive {
            any_inconclusive = true;
        }
        if !result.satisfied {
            all_assumptions_met = false;
            break;
        }
    }

    let satisfied = if !all_assumptions_met {
        // Vacuous satisfaction: false premise -> anything is true
        true
    } else {
        // Step 3: all assumptions met, check constraints
        let mut all_ok = true;
        for constraint in &req.constraints {
            let result = evaluate(constraint, context);
            all_diags.extend(result.diagnostics);
            if result.inconclusive {
                any_inconclusive = true;
            }
            if !result.satisfied {
                all_ok = false;
            }
        }
        all_ok
    };

    // Step 4: apply negation
    let satisfied = if req.is_negated {
        !satisfied
    } else {
        satisfied
    };

    EvaluationResult {
        constraint: ConstraintIR::new(&req.name)
            .with_description(format!("requirement: {}", req.name)),
        satisfied,
        inconclusive: any_inconclusive,
        diagnostics: all_diags,
        operands: HashMap::new(),
    }
}

/// Compile a [`RequirementConstraintIR`] from a SatisfyRequirementUsage element.
///
/// Looks for children of the element that carry an `"assume"` or `"require"`
/// property with a constraint expression, or children with a `"constraint"`
/// property.  The `is_negated` flag is read from the element's `"isNegated"`
/// property.
pub fn compile_satisfy_requirement(
    element: &Element,
    graph: &ModelGraph,
) -> Result<RequirementConstraintIR, Vec<Diagnostic>> {
    if element.kind != ElementKind::SatisfyRequirementUsage
        && element.kind != ElementKind::RequirementUsage
    {
        return Err(vec![Diagnostic::error(format!(
            "expected SatisfyRequirementUsage or RequirementUsage, found {:?}",
            element.kind
        ))]);
    }

    let name = element.name.clone().unwrap_or_else(|| "unnamed".into());
    let is_negated = element
        .get_prop("isNegated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut req = RequirementConstraintIR::new(name);
    if is_negated {
        req.is_negated = true;
    }

    for child in graph.children_of(&element.id) {
        // Children with role "assume" are assumptions
        if let Some(role) = child.get_prop("role").and_then(|v| v.as_str()) {
            // AST-first: pretty-print the structured expression subtree,
            // fall back to legacy string props for test graphs.
            let expr_text = sysml_core::expression_pretty::pretty_print_owner(child, graph)
                .or_else(|| {
                    child
                        .get_prop("constraint")
                        .or_else(|| child.get_prop("expr"))
                        .and_then(|v| v.as_str().map(str::to_owned))
                });
            if let Some(expr) = expr_text {
                let c = ConstraintIR::new(expr)
                    .with_description(child.name.clone().unwrap_or_default());
                if role == "assume" {
                    req.assumptions.push(c);
                } else if role == "require" {
                    req.constraints.push(c);
                }
            }
        }
    }

    Ok(req)
}

// ---------------------------------------------------------------------------
// S2c: Invariant monitoring
// ---------------------------------------------------------------------------

/// A violation detected by the [`ConstraintMonitor`].
///
/// Each violation records which constraint failed, a human-readable message,
/// and an optional source [`Span`] for IDE integration / diagnostics.
#[derive(Debug, Clone)]
pub struct ConstraintViolation {
    /// The constraint that was violated.
    pub constraint: ConstraintIR,
    /// Human-readable description of the violation.
    pub message: String,
    /// Source span where the constraint originates, if available.
    pub span: Option<Span>,
}

/// Continuous constraint monitor.
///
/// An `AssertConstraintUsage` in SysML v2 requires that a constraint hold at
/// every evaluation step. The monitor accumulates constraints and, on each
/// call to [`ConstraintMonitor::check`], evaluates them against a context,
/// recording any violations.
///
/// ## Usage
///
/// ```ignore
/// let mut monitor = ConstraintMonitor::new();
/// monitor.add_constraint(ConstraintIR::new("speed < 100"), None);
/// let violations = monitor.check(&context);
/// assert!(violations.is_empty());
/// ```
#[derive(Debug, Clone)]
pub struct ConstraintMonitor {
    /// Constraints being monitored.
    pub constraints: Vec<(ConstraintIR, Option<Span>)>,
    /// Accumulated violations across all calls to [`check`](Self::check).
    pub violations: Vec<ConstraintViolation>,
}

impl ConstraintMonitor {
    /// Create a new empty monitor.
    pub fn new() -> Self {
        ConstraintMonitor {
            constraints: Vec::new(),
            violations: Vec::new(),
        }
    }

    /// Add a constraint with an optional source span.
    pub fn add_constraint(&mut self, constraint: ConstraintIR, span: Option<Span>) {
        self.constraints.push((constraint, span));
    }

    /// Evaluate all monitored constraints against the given context.
    ///
    /// New violations are both appended to the internal `violations` list and
    /// returned for immediate inspection.
    pub fn check(&mut self, context: &EvalContext) -> Vec<ConstraintViolation> {
        let mut new_violations = Vec::new();
        for (constraint, span) in &self.constraints {
            let result = evaluate(constraint, context);
            if !result.satisfied {
                let message = if result.diagnostics.is_empty() {
                    format!("constraint `{}` violated", constraint.expr)
                } else {
                    result.diagnostics[0].message.clone()
                };
                new_violations.push(ConstraintViolation {
                    constraint: constraint.clone(),
                    message,
                    span: span.clone(),
                });
            }
        }
        self.violations.extend(new_violations.clone());
        new_violations
    }

    /// Return accumulated violations.
    pub fn all_violations(&self) -> &[ConstraintViolation] {
        &self.violations
    }

    /// Clear the accumulated violation history.
    pub fn clear_violations(&mut self) {
        self.violations.clear();
    }
}

impl Default for ConstraintMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Evaluate constraints in batch with optional short-circuit on first failure.
///
/// When `short_circuit` is `true`, evaluation stops at the first failing
/// constraint and returns only that single result. This is useful for
/// performance-sensitive paths where only the *existence* of a failure
/// matters.
pub fn evaluate_batch(
    constraints: &ConstraintSet,
    context: &EvalContext,
    short_circuit: bool,
) -> Vec<EvaluationResult> {
    #[cfg(feature = "tracing")]
    tracing::trace!(
        constraint_count = constraints.constraints.len(),
        binding_count = context.variables.len(),
        short_circuit,
        "evaluating constraint batch"
    );

    let mut results = Vec::new();
    for c in &constraints.constraints {
        let result = evaluate(c, context);
        let failed = !result.satisfied;
        results.push(result);
        if short_circuit && failed {
            break;
        }
    }
    results
}

// ---------------------------------------------------------------------------
// S2d: Integration — state transition constraint checking
// ---------------------------------------------------------------------------

/// Evaluate constraints in the context of a state machine transition.
///
/// This is the cross-crate integration point: a state machine runner calls
/// this function before (or after) a transition fires, passing the transition
/// guard constraints and the current evaluation context.
///
/// Returns `Ok(())` when all constraints are satisfied, or `Err` with the
/// list of violations when any constraint fails.
pub fn check_transition_constraints(
    constraints: &ConstraintSet,
    context: &EvalContext,
) -> Result<(), Vec<ConstraintViolation>> {
    #[cfg(feature = "tracing")]
    tracing::trace!(
        constraint_count = constraints.constraints.len(),
        binding_count = context.variables.len(),
        "checking transition constraints"
    );

    let results = evaluate_all(constraints, context);
    let violations: Vec<ConstraintViolation> = results
        .into_iter()
        .filter(|r| !r.satisfied)
        .map(|r| {
            let message = if r.diagnostics.is_empty() {
                format!("transition guard `{}` not satisfied", r.constraint.expr)
            } else {
                r.diagnostics[0].message.clone()
            };
            ConstraintViolation {
                constraint: r.constraint,
                message,
                span: None,
            }
        })
        .collect();

    if violations.is_empty() {
        #[cfg(feature = "tracing")]
        tracing::trace!("all transition constraints satisfied");
        Ok(())
    } else {
        #[cfg(feature = "tracing")]
        tracing::debug!(
            violation_count = violations.len(),
            "transition constraint check produced violations"
        );
        Err(violations)
    }
}

// ---------------------------------------------------------------------------
// Health diagnostics (CN001–CN005)
// ---------------------------------------------------------------------------

/// Diagnose constraint health issues across all constraints in a graph.
///
/// This pass is intended for editor diagnostics and preflight checks before
/// constraint evaluation. It iterates `ConstraintUsage` and
/// `AssertConstraintUsage` elements and reports compilation failures,
/// undefined variables, violations, non-boolean results, and missing
/// expressions.
pub fn constraint_health_diagnostics(graph: &ModelGraph) -> Vec<Diagnostic> {
    use sysml_core::element_ordering::{primary_span, sort_elements_by_source_order};

    let mut constraints: Vec<_> = graph
        .elements_by_kind(&ElementKind::ConstraintUsage)
        .chain(graph.elements_by_kind(&ElementKind::AssertConstraintUsage))
        .collect();
    sort_elements_by_source_order(&mut constraints);

    let mut diagnostics = Vec::new();
    let evaluator = ExpressionEvaluator::new();

    for elem in constraints {
        // Skip library elements
        if graph.is_library_element(&elem.id) {
            continue;
        }

        let name = elem.name.clone().unwrap_or_else(|| elem.id.to_string());

        // Determine if the element carries *any* expression content
        // (structured AST subtree or, for legacy test graphs, a string
        // prop). CN005 fires only when the constraint body is truly empty.
        // Note: parsers no longer write string props (Phase 6D); the
        // fallback exists solely for hand-crafted test graphs.
        let has_content = sysml_core::expression_pretty::pretty_print_owner(elem, graph).is_some()
            || elem
                .get_prop("constraint")
                .and_then(|v| v.as_str())
                .is_some()
            || elem.get_prop("expr").and_then(|v| v.as_str()).is_some();

        if !has_content {
            // A reference / typed-form constraint usage (`require constraint : Def;`,
            // `constraint c : Def;`, bare `require existingConstraint;`) carries no
            // own body: its predicate is that of the constraint it is typed by or
            // references (SysML §7.20 — "a constraint usage is the usage of a
            // constraint definition"; the usage inherits the definition's Boolean
            // condition, the library ConstraintCheck's BooleanEvaluation). It is NOT
            // "missing an expression", so CN005 must not fire — the case compiler
            // already evaluates it through that FeatureTyping / ReferenceSubsetting
            // (`cases::compile::resolve_referenced_constraint_expr`). An unresolved
            // reference is reported by E200 in resolution, not here.
            let delegates_predicate = graph.children_of(&elem.id).any(|c| {
                matches!(
                    c.kind,
                    ElementKind::FeatureTyping | ElementKind::ReferenceSubsetting
                )
            });
            if delegates_predicate {
                continue;
            }
            // CN005: genuinely bodyless — no own expression and no delegation.
            diagnostics.push(
                Diagnostic::info(format!(
                    "constraint '{}' has no expression to evaluate",
                    name
                ))
                .with_code("CN005")
                .with_span(primary_span(elem)),
            );
            continue;
        }

        // AST-first compilation via compile_expression; the dispatcher
        // falls back to string props when the AST walk is empty.
        let ir = match compile_expression(elem, graph) {
            Ok(ir) => ir,
            Err(diags) => {
                // CN001: compilation failure
                let detail = diags
                    .first()
                    .map(|d| d.message.as_str())
                    .unwrap_or("unknown error");
                diagnostics.push(
                    Diagnostic::warning(format!(
                        "constraint '{}': expression cannot be compiled: {}",
                        name, detail
                    ))
                    .with_code("CN001")
                    .with_span(primary_span(elem)),
                );
                continue;
            }
        };

        // Display-only string for downstream diagnostic messages.
        let expr_str: String = sysml_core::expression_pretty::pretty_print_owner(elem, graph)
            .unwrap_or_else(|| format!("{:?}", ir));

        // Try to evaluate with empty context (best-effort)
        let ctx = EvalContext::new();
        match evaluator.eval(&ir, &ctx) {
            Ok(Value::Bool(true)) => { /* pass — no diagnostic */ }
            Ok(Value::Bool(false)) => {
                // CN003: constraint violated
                diagnostics.push(
                    Diagnostic::warning(format!("constraint '{}' violated: {}", name, expr_str))
                        .with_code("CN003")
                        .with_span(primary_span(elem)),
                );
            }
            Ok(other) => {
                // CN004: non-boolean result
                diagnostics.push(
                    Diagnostic::info(format!(
                        "constraint '{}': evaluates to {}, expected boolean",
                        name,
                        other.type_name()
                    ))
                    .with_code("CN004")
                    .with_span(primary_span(elem)),
                );
            }
            Err(EvaluationError::UndefinedVariable(var)) => {
                // CN002: undefined variable (inconclusive)
                diagnostics.push(
                    Diagnostic::info(format!(
                        "constraint '{}': variable '{}' has no value — result is inconclusive",
                        name, var
                    ))
                    .with_code("CN002")
                    .with_span(primary_span(elem)),
                );
            }
            Err(_) => {
                // Other eval error — skip (don't spam diagnostics for complex expressions)
            }
        }
    }

    diagnostics
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn value_to_scalar_reads_quantity_and_complex_magnitude() {
        use sysml_core::physics::DimensionVector;
        // Regression (AUDIT-2026-06-01 WS1 Step 0): the analysis-ir copy of
        // value_to_scalar dropped Quantity/Complex, silently skipping threshold
        // constraints on physics-typed features. Both must now yield a scalar.
        let q = Value::Quantity {
            value: 5.0,
            dimension: DimensionVector::default(),
            unit: Some("A".into()),
        };
        assert_eq!(value_to_scalar(&q), Some(5.0));
        let c = Value::Complex { re: 3.0, im: 4.0 };
        assert_eq!(value_to_scalar(&c), Some(3.0));
        // Non-numeric values still map to None.
        assert_eq!(value_to_scalar(&Value::String("x".into())), None);
    }

    #[test]
    fn constraint_set_creation() {
        let mut set = ConstraintSet::new();
        set.add(ConstraintIR::new("x < 10"));
        set.add(ConstraintIR::new("y > 0"));

        assert_eq!(set.len(), 2);
    }

    #[test]
    fn extract_constraints_from_graph() {
        let mut graph = ModelGraph::new();

        let elem = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("SpeedLimit")
            .with_prop("constraint", "speed < 100");
        graph.add_element(elem);

        let constraints = extract_constraints(&graph);
        assert_eq!(constraints.len(), 1);
        assert_eq!(constraints.constraints[0].expr, "speed < 100");
    }

    #[test]
    fn evaluate_populates_live_operands() {
        // GAP-CONSTR-002: every free variable in the expression shows
        // up in `operands` with its current f64 value.
        let constraint = ConstraintIR::new("temperature < cap");
        let mut context = EvalContext::new();
        context.set("temperature", Value::Float(321.5));
        context.set("cap", Value::Int(400));
        // Unrelated variable should NOT appear in operands.
        context.set("unrelated", Value::Float(99.0));

        let result = evaluate(&constraint, &context);
        assert!(result.satisfied);
        assert_eq!(result.operands.get("temperature"), Some(&321.5));
        assert_eq!(result.operands.get("cap"), Some(&400.0));
        assert!(!result.operands.contains_key("unrelated"));
    }

    #[test]
    fn evaluate_operands_skip_non_scalars_and_missing() {
        // String / list / missing variables all drop out silently —
        // the UI can only render badges for numeric operands.
        let constraint = ConstraintIR::new("x > 0");
        let mut context = EvalContext::new();
        context.set("x", Value::String("not numeric".into()));

        let result = evaluate(&constraint, &context);
        assert!(result.inconclusive || !result.satisfied);
        assert!(!result.operands.contains_key("x"));
    }

    #[test]
    fn evaluate_less_than() {
        let constraint = ConstraintIR::new("x < 10");
        let mut context = EvalContext::new();
        context.set("x", Value::Float(5.0));

        let result = evaluate(&constraint, &context);
        assert!(result.satisfied);

        context.set("x", Value::Float(15.0));
        let result = evaluate(&constraint, &context);
        assert!(!result.satisfied);
    }

    #[test]
    fn evaluate_greater_than() {
        let constraint = ConstraintIR::new("x > 0");
        let mut context = EvalContext::new();
        context.set("x", Value::Float(5.0));

        let result = evaluate(&constraint, &context);
        assert!(result.satisfied);

        context.set("x", Value::Float(-1.0));
        let result = evaluate(&constraint, &context);
        assert!(!result.satisfied);
    }

    #[test]
    fn evaluate_equals() {
        let constraint = ConstraintIR::new("x == 10");
        let mut context = EvalContext::new();
        context.set("x", Value::Int(10));

        let result = evaluate(&constraint, &context);
        assert!(result.satisfied);

        context.set("x", Value::Int(11));
        let result = evaluate(&constraint, &context);
        assert!(!result.satisfied);
    }

    #[test]
    fn evaluate_all_constraints() {
        let mut set = ConstraintSet::new();
        set.add(ConstraintIR::new("x < 100"));
        set.add(ConstraintIR::new("y > 0"));

        let mut context = EvalContext::new();
        context.set("x", Value::Float(50.0));
        context.set("y", Value::Float(10.0));

        let results = evaluate_all(&set, &context);
        assert!(all_satisfied(&results));
    }

    #[test]
    fn failed_constraints_detection() {
        let mut set = ConstraintSet::new();
        set.add(ConstraintIR::new("x < 100"));
        set.add(ConstraintIR::new("y > 0"));

        let mut context = EvalContext::new();
        context.set("x", Value::Float(50.0));
        context.set("y", Value::Float(-5.0));

        let results = evaluate_all(&set, &context);
        assert!(!all_satisfied(&results));

        let failed = failed_constraints(&results);
        assert_eq!(failed.len(), 1);
    }

    #[test]
    fn evaluate_with_int_values() {
        let constraint = ConstraintIR::new("count < 5");
        let mut context = EvalContext::new();
        context.set("count", Value::Int(3));

        let result = evaluate(&constraint, &context);
        assert!(result.satisfied);
    }

    #[test]
    fn invalid_expression_returns_diagnostic() {
        let constraint = ConstraintIR::new("@#$ invalid");
        let context = EvalContext::new();

        let result = evaluate(&constraint, &context);
        assert!(!result.satisfied);
        assert!(result.inconclusive);
        assert!(!result.diagnostics.is_empty());
    }

    // -- F031: Undefined variable produces inconclusive, not false -----------

    #[test]
    fn undefined_variable_is_inconclusive() {
        // An undefined variable should NOT produce satisfied=false with no
        // indication that the result is actually "unknown". It must set
        // inconclusive=true so callers can distinguish "evaluated and failed"
        // from "could not evaluate".
        let constraint = ConstraintIR::new("x > 0");
        let context = EvalContext::new(); // x not defined

        let result = evaluate(&constraint, &context);
        assert!(!result.satisfied, "undefined var should not be satisfied");
        assert!(
            result.inconclusive,
            "undefined var should be inconclusive, not a definitive failure"
        );
        assert!(
            !result.diagnostics.is_empty(),
            "should have a diagnostic about the undefined variable"
        );
    }

    #[test]
    fn defined_variable_is_not_inconclusive() {
        let constraint = ConstraintIR::new("x > 0");
        let mut context = EvalContext::new();
        context.set("x", Value::Float(5.0));

        let result = evaluate(&constraint, &context);
        assert!(result.satisfied);
        assert!(
            !result.inconclusive,
            "defined variable evaluation should not be inconclusive"
        );
    }

    #[test]
    fn failed_constraint_is_not_inconclusive() {
        let constraint = ConstraintIR::new("x > 10");
        let mut context = EvalContext::new();
        context.set("x", Value::Float(5.0));

        let result = evaluate(&constraint, &context);
        assert!(!result.satisfied);
        assert!(
            !result.inconclusive,
            "a constraint that evaluates to false is NOT inconclusive"
        );
    }

    #[test]
    fn inconclusive_constraints_helper() {
        let mut set = ConstraintSet::new();
        set.add(ConstraintIR::new("x > 0")); // x undefined -> inconclusive
        set.add(ConstraintIR::new("y > 0")); // y undefined -> inconclusive

        let context = EvalContext::new();
        let results = evaluate_all(&set, &context);

        assert_eq!(
            inconclusive_constraints(&results).len(),
            2,
            "both constraints should be inconclusive"
        );
        assert!(
            failed_constraints(&results).is_empty(),
            "no constraint should be in the 'failed' list (they are inconclusive, not failed)"
        );
    }

    // -- S2a: Collection constraints ----------------------------------------

    #[test]
    fn evaluate_expr_ir_directly() {
        use crate::expressions::BinOp;

        // Build an ExprIR programmatically instead of compiling from string
        let expr = ExprIR::BinaryOp {
            op: BinOp::LessThan,
            left: Box::new(ExprIR::FeatureRef("speed".into())),
            right: Box::new(ExprIR::LiteralReal(100.0)),
        };

        let constraint = ConstraintIR::new("speed < 100");
        let mut context = EvalContext::new();
        context.set("speed", Value::Float(85.0));

        let result = evaluate_expr(&expr, &constraint, &context);
        assert!(result.satisfied);
        assert!(result.diagnostics.is_empty());

        // Verify it also fails when the constraint is violated
        context.set("speed", Value::Float(120.0));
        let result = evaluate_expr(&expr, &constraint, &context);
        assert!(!result.satisfied);
    }

    #[test]
    fn constraint_with_typed_expr() {
        use crate::expressions::BinOp;

        // Create via compile()
        let typed = TypedConstraint::compile(ConstraintIR::new("x > 0")).unwrap();
        let mut context = EvalContext::new();
        context.set("x", Value::Float(5.0));
        let result = typed.evaluate(&context);
        assert!(result.satisfied);

        // Create via from_expr()
        let expr = ExprIR::BinaryOp {
            op: BinOp::Equal,
            left: Box::new(ExprIR::FeatureRef("y".into())),
            right: Box::new(ExprIR::LiteralInt(42)),
        };
        let typed = TypedConstraint::from_expr(expr, Some("y must be 42".to_string()));
        let mut context = EvalContext::new();
        context.set("y", Value::Int(42));
        let result = typed.evaluate(&context);
        assert!(result.satisfied);
        assert_eq!(
            typed.constraint.description.as_deref(),
            Some("y must be 42")
        );
    }

    #[test]
    fn forall_constraint_pass() {
        use crate::expressions::BinOp;

        // forAll: all items > 0
        let expr = ExprIR::ForAll {
            source: Box::new(ExprIR::FeatureRef("items".into())),
            binding: "x".into(),
            predicate: Box::new(ExprIR::BinaryOp {
                op: BinOp::GreaterThan,
                left: Box::new(ExprIR::FeatureRef("x".into())),
                right: Box::new(ExprIR::LiteralInt(0)),
            }),
        };

        let constraint = ConstraintIR::new("items->forAll { |x| x > 0 }");
        let mut context = EvalContext::new();
        context.set(
            "items",
            Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        );

        let result = evaluate_expr(&expr, &constraint, &context);
        assert!(result.satisfied);
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn forall_constraint_fail() {
        use crate::expressions::BinOp;

        // forAll: all items > 0 — but one is negative
        let expr = ExprIR::ForAll {
            source: Box::new(ExprIR::FeatureRef("items".into())),
            binding: "x".into(),
            predicate: Box::new(ExprIR::BinaryOp {
                op: BinOp::GreaterThan,
                left: Box::new(ExprIR::FeatureRef("x".into())),
                right: Box::new(ExprIR::LiteralInt(0)),
            }),
        };

        let constraint = ConstraintIR::new("items->forAll { |x| x > 0 }");
        let mut context = EvalContext::new();
        context.set(
            "items",
            Value::List(vec![Value::Int(1), Value::Int(-2), Value::Int(3)]),
        );

        let result = evaluate_expr(&expr, &constraint, &context);
        assert!(!result.satisfied);
    }

    #[test]
    fn exists_constraint_found() {
        use crate::expressions::BinOp;

        // exists: at least one item == 5
        let expr = ExprIR::Exists {
            source: Box::new(ExprIR::FeatureRef("items".into())),
            binding: "x".into(),
            predicate: Box::new(ExprIR::BinaryOp {
                op: BinOp::Equal,
                left: Box::new(ExprIR::FeatureRef("x".into())),
                right: Box::new(ExprIR::LiteralInt(5)),
            }),
        };

        let constraint = ConstraintIR::new("items->exists { |x| x == 5 }");
        let mut context = EvalContext::new();
        context.set(
            "items",
            Value::List(vec![Value::Int(1), Value::Int(5), Value::Int(9)]),
        );

        let result = evaluate_expr(&expr, &constraint, &context);
        assert!(result.satisfied);
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn exists_constraint_not_found() {
        use crate::expressions::BinOp;

        // exists: looking for 99 which is not in the list
        let expr = ExprIR::Exists {
            source: Box::new(ExprIR::FeatureRef("items".into())),
            binding: "x".into(),
            predicate: Box::new(ExprIR::BinaryOp {
                op: BinOp::Equal,
                left: Box::new(ExprIR::FeatureRef("x".into())),
                right: Box::new(ExprIR::LiteralInt(99)),
            }),
        };

        let constraint = ConstraintIR::new("items->exists { |x| x == 99 }");
        let mut context = EvalContext::new();
        context.set(
            "items",
            Value::List(vec![Value::Int(1), Value::Int(5), Value::Int(9)]),
        );

        let result = evaluate_expr(&expr, &constraint, &context);
        assert!(!result.satisfied);
    }

    // -- S2b: Requirement satisfaction ----------------------------------------

    #[test]
    fn requirement_all_assumptions_met() {
        // When all assumptions are true and all constraints are true,
        // the requirement is satisfied.
        let req = RequirementConstraintIR::new("SpeedReq")
            .with_assumption(ConstraintIR::new("speed > 0"))
            .with_constraint(ConstraintIR::new("speed < 100"));

        let mut ctx = EvalContext::new();
        ctx.set("speed", Value::Float(50.0));

        let result = evaluate_requirement(&req, &ctx);
        assert!(result.satisfied);
        assert!(result.diagnostics.is_empty());

        // Now violate the constraint
        ctx.set("speed", Value::Float(150.0));
        let result = evaluate_requirement(&req, &ctx);
        assert!(!result.satisfied);
    }

    #[test]
    fn requirement_vacuous_satisfaction() {
        // When an assumption is false, the requirement is vacuously satisfied.
        let req = RequirementConstraintIR::new("VacuousReq")
            .with_assumption(ConstraintIR::new("active == 1"))
            .with_constraint(ConstraintIR::new("speed < 100"));

        let mut ctx = EvalContext::new();
        ctx.set("active", Value::Int(0)); // assumption fails
        ctx.set("speed", Value::Float(999.0)); // constraint would fail

        let result = evaluate_requirement(&req, &ctx);
        assert!(result.satisfied); // vacuously satisfied
    }

    #[test]
    fn compile_satisfy_requirement_from_graph() {
        let mut graph = ModelGraph::new();

        // Create a SatisfyRequirementUsage element
        let req_elem =
            Element::new_with_kind(ElementKind::SatisfyRequirementUsage).with_name("SafeSpeed");
        let req_id = graph.add_element(req_elem);

        // Add an assumption child
        let assume_child = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("EngineRunning")
            .with_owner(req_id.clone())
            .with_prop("role", "assume")
            .with_prop("constraint", "engine_on == 1");
        graph.add_element(assume_child);

        // Add a required constraint child
        let require_child = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("SpeedLimit")
            .with_owner(req_id.clone())
            .with_prop("role", "require")
            .with_prop("constraint", "speed < 200");
        graph.add_element(require_child);

        let req_elem = graph.get_element(&req_id).unwrap();
        let compiled = compile_satisfy_requirement(req_elem, &graph).unwrap();

        assert_eq!(compiled.name, "SafeSpeed");
        assert_eq!(compiled.assumptions.len(), 1);
        assert_eq!(compiled.constraints.len(), 1);
        assert!(!compiled.is_negated);

        // Evaluate with assumptions met and constraint satisfied
        let mut ctx = EvalContext::new();
        ctx.set("engine_on", Value::Int(1));
        ctx.set("speed", Value::Float(100.0));

        let result = evaluate_requirement(&compiled, &ctx);
        assert!(result.satisfied);
    }

    #[test]
    fn negated_requirement_inverts_result() {
        // A negated requirement inverts: satisfied becomes unsatisfied.
        let req = RequirementConstraintIR::new("NegReq")
            .with_assumption(ConstraintIR::new("x > 0"))
            .with_constraint(ConstraintIR::new("x < 10"))
            .negated();

        let mut ctx = EvalContext::new();
        ctx.set("x", Value::Float(5.0));

        // Normally this would be satisfied (assumption true, constraint true),
        // but negation inverts it.
        let result = evaluate_requirement(&req, &ctx);
        assert!(!result.satisfied);

        // When the constraint fails (x >= 10), negation makes it satisfied.
        ctx.set("x", Value::Float(15.0));
        let result = evaluate_requirement(&req, &ctx);
        assert!(result.satisfied);
    }

    // -- S2c: Invariant monitoring -------------------------------------------

    #[test]
    fn assert_constraint_checked_each_step() {
        // Monitor evaluates constraints on each check() call, accumulating
        // violations over time (like AssertConstraintUsage continuous checking).
        let mut monitor = ConstraintMonitor::new();
        monitor.add_constraint(ConstraintIR::new("speed < 100"), None);

        let mut ctx = EvalContext::new();

        // Step 1: speed OK
        ctx.set("speed", Value::Float(50.0));
        let v = monitor.check(&ctx);
        assert!(v.is_empty());
        assert_eq!(monitor.all_violations().len(), 0);

        // Step 2: speed still OK
        ctx.set("speed", Value::Float(80.0));
        let v = monitor.check(&ctx);
        assert!(v.is_empty());
        assert_eq!(monitor.all_violations().len(), 0);

        // Step 3: speed exceeds limit — violation recorded
        ctx.set("speed", Value::Float(120.0));
        let v = monitor.check(&ctx);
        assert_eq!(v.len(), 1);
        assert_eq!(monitor.all_violations().len(), 1);

        // Step 4: speed back to normal — previous violation still recorded
        ctx.set("speed", Value::Float(60.0));
        let v = monitor.check(&ctx);
        assert!(v.is_empty());
        assert_eq!(monitor.all_violations().len(), 1); // cumulative
    }

    #[test]
    fn violation_includes_span() {
        let mut monitor = ConstraintMonitor::new();
        let span = Span::new("vehicle.sysml", 42, 80);
        monitor.add_constraint(ConstraintIR::new("speed < 100"), Some(span.clone()));

        let mut ctx = EvalContext::new();
        ctx.set("speed", Value::Float(150.0));

        let violations = monitor.check(&ctx);
        assert_eq!(violations.len(), 1);
        let v = &violations[0];
        assert!(v.span.is_some());
        let s = v.span.as_ref().unwrap();
        assert_eq!(s.file, "vehicle.sysml");
        assert_eq!(s.start, 42);
        assert_eq!(s.end, 80);
    }

    #[test]
    fn batch_stops_on_first_failure_optional() {
        let mut set = ConstraintSet::new();
        set.add(ConstraintIR::new("x > 0")); // will pass
        set.add(ConstraintIR::new("y > 0")); // will fail
        set.add(ConstraintIR::new("z > 0")); // would fail but short-circuit skips

        let mut ctx = EvalContext::new();
        ctx.set("x", Value::Float(5.0));
        ctx.set("y", Value::Float(-1.0));
        ctx.set("z", Value::Float(-2.0));

        // Without short-circuit: evaluates all
        let results = evaluate_batch(&set, &ctx, false);
        assert_eq!(results.len(), 3);
        assert!(results[0].satisfied);
        assert!(!results[1].satisfied);
        assert!(!results[2].satisfied);

        // With short-circuit: stops after first failure (y > 0)
        let results = evaluate_batch(&set, &ctx, true);
        assert_eq!(results.len(), 2); // x (pass), y (fail), z skipped
        assert!(results[0].satisfied);
        assert!(!results[1].satisfied);
    }

    #[test]
    fn monitor_multiple_constraints() {
        let mut monitor = ConstraintMonitor::new();
        monitor.add_constraint(ConstraintIR::new("x > 0"), None);
        monitor.add_constraint(ConstraintIR::new("y > 0"), None);

        let mut ctx = EvalContext::new();
        ctx.set("x", Value::Float(5.0));
        ctx.set("y", Value::Float(-1.0));

        let violations = monitor.check(&ctx);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].constraint.expr.contains("y > 0"));
    }

    #[test]
    fn monitor_clear_violations() {
        let mut monitor = ConstraintMonitor::new();
        monitor.add_constraint(ConstraintIR::new("x > 0"), None);

        let mut ctx = EvalContext::new();
        ctx.set("x", Value::Float(-1.0));
        monitor.check(&ctx);
        assert_eq!(monitor.all_violations().len(), 1);

        monitor.clear_violations();
        assert!(monitor.all_violations().is_empty());
    }

    #[test]
    fn monitor_default_trait() {
        let monitor = ConstraintMonitor::default();
        assert!(monitor.constraints.is_empty());
        assert!(monitor.violations.is_empty());
    }

    // -- S2d: Integration — transition constraint checking -------------------

    #[test]
    fn statemachine_checks_constraint_at_transition() {
        // Simulate: state machine in "Idle" state, transition to "Running"
        // has guards: engine_on == 1, fuel > 0
        let mut guards = ConstraintSet::new();
        guards.add(
            ConstraintIR::new("engine_on == 1").with_description("engine must be on".to_string()),
        );
        guards.add(ConstraintIR::new("fuel > 0").with_description("need fuel".to_string()));

        // Context: engine on, fuel available => transition allowed
        let mut ctx = EvalContext::new();
        ctx.set("engine_on", Value::Int(1));
        ctx.set("fuel", Value::Float(50.0));

        let result = check_transition_constraints(&guards, &ctx);
        assert!(result.is_ok());

        // Context: engine off => transition blocked
        ctx.set("engine_on", Value::Int(0));
        let result = check_transition_constraints(&guards, &ctx);
        assert!(result.is_err());
        let violations = result.unwrap_err();
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("engine_on == 1"));
    }

    #[test]
    fn transition_all_guards_fail() {
        let mut guards = ConstraintSet::new();
        guards.add(ConstraintIR::new("a > 0"));
        guards.add(ConstraintIR::new("b > 0"));

        let mut ctx = EvalContext::new();
        ctx.set("a", Value::Float(-1.0));
        ctx.set("b", Value::Float(-1.0));

        let result = check_transition_constraints(&guards, &ctx);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().len(), 2);
    }

    #[test]
    fn transition_empty_constraints_ok() {
        let guards = ConstraintSet::new();
        let ctx = EvalContext::new();
        let result = check_transition_constraints(&guards, &ctx);
        assert!(result.is_ok());
    }

    // -- B1: Pre-compilation ---------------------------------------------------

    #[test]
    fn precompile_valid_constraints() {
        let mut set = ConstraintSet::new();
        set.add(ConstraintIR::new("x < 10"));
        set.add(ConstraintIR::new("y > 0"));

        let precompiled = precompile_constraint_set(&set);
        assert_eq!(precompiled.compiled_count(), 2);
        assert_eq!(precompiled.failed_count(), 0);
        assert!(precompiled.diagnostics().is_empty());
    }

    #[test]
    fn precompile_catches_invalid_expression() {
        let mut set = ConstraintSet::new();
        set.add(ConstraintIR::new("x < 10"));
        set.add(ConstraintIR::new("@#$ invalid"));

        let precompiled = precompile_constraint_set(&set);
        assert_eq!(precompiled.compiled_count(), 1);
        assert_eq!(precompiled.failed_count(), 1);
        assert!(!precompiled.diagnostics().is_empty());
    }

    #[test]
    fn precompile_evaluate_all() {
        let mut set = ConstraintSet::new();
        set.add(ConstraintIR::new("x < 100"));
        set.add(ConstraintIR::new("y > 0"));

        let precompiled = precompile_constraint_set(&set);
        assert_eq!(precompiled.compiled_count(), 2);

        let mut ctx = EvalContext::new();
        ctx.set("x", Value::Float(50.0));
        ctx.set("y", Value::Float(10.0));

        let results = precompiled.evaluate_all(&ctx);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.satisfied));
    }

    #[test]
    fn extract_and_precompile_from_graph() {
        let mut graph = ModelGraph::new();

        let elem = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("SpeedLimit")
            .with_prop("constraint", "speed < 100");
        graph.add_element(elem);

        let precompiled = extract_and_precompile(&graph);
        assert_eq!(precompiled.compiled_count(), 1);
        assert_eq!(precompiled.failed_count(), 0);

        let mut ctx = EvalContext::new();
        ctx.set("speed", Value::Float(50.0));
        let results = precompiled.evaluate_all(&ctx);
        assert!(results[0].satisfied);
    }

    // -- M1: Cross-crate integration — Expressions → Constraints ---------------

    #[test]
    fn constraint_with_complex_expression() {
        // Verify that a constraint using compound arithmetic from the expression
        // evaluator (sysml-run-expressions) works end-to-end through the
        // constraint evaluation pipeline.
        //
        // Expression: x + y * 2 > threshold
        // With x=5, y=10, threshold=20: 5 + 10*2 = 25 > 20 → true → satisfied
        use crate::expressions::BinOp;

        let expr = ExprIR::BinaryOp {
            op: BinOp::GreaterThan,
            left: Box::new(ExprIR::BinaryOp {
                op: BinOp::Add,
                left: Box::new(ExprIR::FeatureRef("x".into())),
                right: Box::new(ExprIR::BinaryOp {
                    op: BinOp::Multiply,
                    left: Box::new(ExprIR::FeatureRef("y".into())),
                    right: Box::new(ExprIR::LiteralInt(2)),
                }),
            }),
            right: Box::new(ExprIR::FeatureRef("threshold".into())),
        };

        let constraint = ConstraintIR::new("x + y * 2 > threshold")
            .with_description("complex arithmetic guard".to_string());

        let mut ctx = EvalContext::new();
        ctx.set("x", Value::Int(5));
        ctx.set("y", Value::Int(10));
        ctx.set("threshold", Value::Int(20));

        // 5 + 10*2 = 25 > 20 → satisfied
        let result = evaluate_expr(&expr, &constraint, &ctx);
        assert!(result.satisfied, "5 + 10*2 = 25 > 20 should be satisfied");
        assert!(result.diagnostics.is_empty());

        // Also test via TypedConstraint for the pre-compiled path
        let typed = TypedConstraint::from_expr(expr.clone(), Some("complex guard".to_string()));
        let result = typed.evaluate(&ctx);
        assert!(result.satisfied);

        // Now with values that make the expression false:
        // x=1, y=2, threshold=20 → 1 + 2*2 = 5 > 20 → false
        ctx.set("x", Value::Int(1));
        ctx.set("y", Value::Int(2));
        ctx.set("threshold", Value::Int(20));

        let result = evaluate_expr(&expr, &constraint, &ctx);
        assert!(
            !result.satisfied,
            "1 + 2*2 = 5 > 20 should NOT be satisfied"
        );

        // Verify that a ConstraintSet + evaluate_all also works with complex
        // constraints alongside simple ones
        let mut set = ConstraintSet::new();
        set.add(ConstraintIR::new("x < 100"));
        set.add(ConstraintIR::new("y > 0"));

        ctx.set("x", Value::Int(1));
        ctx.set("y", Value::Int(2));

        let results = evaluate_all(&set, &ctx);
        assert!(all_satisfied(&results));
    }

    // -----------------------------------------------------------------------
    // C2: End-to-end constraint evaluation pipeline tests
    // -----------------------------------------------------------------------

    /// Full pipeline: constraint definitions -> elaborate -> extract_and_precompile
    /// -> evaluate with context -> verify pass/fail.
    #[test]
    fn e2e_constraint_pipeline_basic() {
        use sysml_core::elaborate::elaborate;

        let mut graph = ModelGraph::new();

        // Create constraint: speed < 100
        let c1 = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("SpeedLimit")
            .with_prop("unresolved_value", "speed < 100");
        graph.add_element(c1);

        // Create constraint: temp > 0
        let c2 = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("AboveZero")
            .with_prop("unresolved_value", "temp > 0");
        graph.add_element(c2);

        // Step 1: Elaborate (copies unresolved_value to constraint/expr)
        let report = elaborate(&mut graph);
        assert!(report.constraints_derived >= 2);

        // Step 2: Extract and precompile.
        // Elaboration writes only `constraint` on ConstraintUsage (no
        // duplicate `expr`), so extract_and_precompile yields one entry
        // per element.
        let precompiled = extract_and_precompile(&graph);
        assert_eq!(precompiled.compiled_count(), 2);
        assert_eq!(precompiled.failed_count(), 0);

        // Step 3: Evaluate — constraints satisfied
        let mut ctx = EvalContext::new();
        ctx.set("speed", Value::Int(80));
        ctx.set("temp", Value::Int(25));

        let results = precompiled.evaluate_all(&ctx);
        assert!(
            results.iter().all(|r| r.satisfied),
            "speed=80, temp=25 should satisfy both constraints"
        );

        // Step 4: Evaluate — first constraint violated.
        ctx.set("speed", Value::Int(120));
        let results = precompiled.evaluate_all(&ctx);
        let failed: Vec<_> = results.iter().filter(|r| !r.satisfied).collect();
        assert_eq!(failed.len(), 1, "speed=120 should violate SpeedLimit");
    }

    /// Pipeline with assume/require roles on a RequirementUsage.
    #[test]
    fn e2e_requirement_assume_require() {
        use sysml_core::elaborate::elaborate;

        let mut graph = ModelGraph::new();

        // Create a requirement with an assumption and a constraint
        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("SafetyReq");
        let req_id = graph.add_element(req);

        // Assumption: the system is powered (power == true)
        let assumption = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("PoweredOn")
            .with_owner(req_id.clone())
            .with_prop("constraintKind", "assumption")
            .with_prop("unresolved_value", "power == true");
        graph.add_element(assumption);

        // Requirement: speed must be below 100
        let requirement = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("SpeedLimit")
            .with_owner(req_id.clone())
            .with_prop("constraintKind", "requirement")
            .with_prop("unresolved_value", "speed < 100");
        graph.add_element(requirement);

        // Elaborate
        elaborate(&mut graph);

        // Build requirement IR from the graph
        let req_elem = graph.get_element(&req_id).unwrap();
        let req_ir = compile_satisfy_requirement(req_elem, &graph).unwrap();

        assert_eq!(req_ir.assumptions.len(), 1);
        assert_eq!(req_ir.constraints.len(), 1);
        assert!(!req_ir.is_negated);

        // Case 1: assumption met, constraint met -> satisfied
        let mut ctx = EvalContext::new();
        ctx.set("power", Value::Bool(true));
        ctx.set("speed", Value::Int(60));

        let result = evaluate_requirement(&req_ir, &ctx);
        assert!(result.satisfied, "powered + speed<100 should satisfy");

        // Case 2: assumption met, constraint violated -> not satisfied
        ctx.set("speed", Value::Int(120));
        let result = evaluate_requirement(&req_ir, &ctx);
        assert!(!result.satisfied, "powered + speed=120 should fail");

        // Case 3: assumption not met -> vacuously satisfied
        ctx.set("power", Value::Bool(false));
        ctx.set("speed", Value::Int(999));
        let result = evaluate_requirement(&req_ir, &ctx);
        assert!(
            result.satisfied,
            "unpowered -> vacuously satisfied regardless of speed"
        );
    }

    /// Pipeline with negated assert constraint.
    #[test]
    fn e2e_negated_assert_constraint() {
        use sysml_core::elaborate::elaborate;

        let mut graph = ModelGraph::new();

        // Create a negated assert: assert not (speed > 100)
        // This means: the system asserts that speed > 100 is NOT true
        let c = Element::new_with_kind(ElementKind::AssertConstraintUsage)
            .with_name("NoExcessSpeed")
            .with_prop("isNegated", "true") // parser sets as string
            .with_prop("unresolved_value", "speed > 100");
        let c_id = graph.add_element(c);

        // Elaborate: normalises isNegated to bool, copies value
        elaborate(&mut graph);

        let elem = graph.get_element(&c_id).unwrap();
        assert_eq!(
            elem.get_prop("isNegated").and_then(|v| v.as_bool()),
            Some(true)
        );

        // Extract and precompile
        let precompiled = extract_and_precompile(&graph);
        assert!(precompiled.compiled_count() >= 1);

        // The extracted constraint expression is "speed > 100". GAP-1 fix
        // (SysML §7.20): `extract_and_precompile` reads the element's
        // isNegated and sets `ConstraintIR.is_negated`, so the decided verdict
        // is INVERTED at the eval layer — negation is no longer deferred to the
        // requirement evaluator.
        let mut ctx = EvalContext::new();

        // speed=50: inner "speed > 100" is false; negated -> the assertion is
        // SATISFIED (the system asserts speed never exceeds 100, and it doesn't).
        ctx.set("speed", Value::Int(50));
        let results = precompiled.evaluate_all(&ctx);
        let speed_result = &results[0];
        assert!(
            speed_result.satisfied && !speed_result.inconclusive,
            "speed=50: negated 'speed > 100' (inner false) must be SATISFIED at the eval layer"
        );

        // speed=120: inner "speed > 100" is true; negated -> VIOLATED.
        ctx.set("speed", Value::Int(120));
        let results = precompiled.evaluate_all(&ctx);
        assert!(
            !results[0].satisfied && !results[0].inconclusive,
            "speed=120: negated 'speed > 100' (inner true) must be VIOLATED at the eval layer"
        );

        // The same negation also works through RequirementConstraintIR (the
        // requirement-level negation path, unchanged by GAP-1).
        let neg_req = RequirementConstraintIR::new("NoExcessSpeed")
            .with_constraint(ConstraintIR::new("speed > 100"))
            .negated();

        // speed=50: constraint "speed > 100" -> false, all_constraints = false,
        // negated -> true (assertion passes)
        ctx.set("speed", Value::Int(50));
        let result = evaluate_requirement(&neg_req, &ctx);
        assert!(
            result.satisfied,
            "speed=50: negated 'speed > 100' should be satisfied"
        );

        // speed=120: constraint "speed > 100" -> true, all_constraints = true,
        // negated -> false (assertion fails)
        ctx.set("speed", Value::Int(120));
        let result = evaluate_requirement(&neg_req, &ctx);
        assert!(
            !result.satisfied,
            "speed=120: negated 'speed > 100' should NOT be satisfied"
        );
    }

    /// Pipeline with precompiled constraints using new expression forms.
    #[test]
    fn e2e_constraint_with_new_expressions() {
        use sysml_core::elaborate::elaborate;

        let mut graph = ModelGraph::new();

        // Conditional expression: if mode == 1 ? speed < 50 else speed < 100
        let c = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("ModeCheck")
            .with_prop(
                "unresolved_value",
                "if mode == 1 ? speed < 50 else speed < 100",
            );
        graph.add_element(c);

        elaborate(&mut graph);

        // One constraint element → one compiled entry.
        let precompiled = extract_and_precompile(&graph);
        assert_eq!(precompiled.compiled_count(), 1);
        assert_eq!(precompiled.failed_count(), 0);

        let mut ctx = EvalContext::new();

        // Mode 1, speed 40 -> 40 < 50 -> true
        ctx.set("mode", Value::Int(1));
        ctx.set("speed", Value::Int(40));
        let results = precompiled.evaluate_all(&ctx);
        assert!(results.iter().all(|r| r.satisfied));

        // Mode 1, speed 60 -> 60 < 50 -> false
        ctx.set("speed", Value::Int(60));
        let results = precompiled.evaluate_all(&ctx);
        assert!(results.iter().all(|r| !r.satisfied));

        // Mode 2, speed 80 -> 80 < 100 -> true
        ctx.set("mode", Value::Int(2));
        ctx.set("speed", Value::Int(80));
        let results = precompiled.evaluate_all(&ctx);
        assert!(results.iter().all(|r| r.satisfied));
    }

    // -- CN001–CN005: constraint health diagnostics ----------------------------

    #[test]
    fn health_cn005_no_expression() {
        let mut graph = ModelGraph::new();
        // Constraint with no expression props at all
        let elem =
            Element::new_with_kind(ElementKind::ConstraintUsage).with_name("EmptyConstraint");
        graph.add_element(elem);

        let diags = constraint_health_diagnostics(&graph);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code.as_deref(), Some("CN005"));
        assert!(diags[0].message.contains("no expression to evaluate"));
    }

    #[test]
    fn health_cn001_compilation_failure() {
        let mut graph = ModelGraph::new();
        let elem = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("BadExpr")
            .with_prop("constraint", "@#$ invalid");
        graph.add_element(elem);

        let diags = constraint_health_diagnostics(&graph);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code.as_deref(), Some("CN001"));
        assert!(diags[0].message.contains("cannot be compiled"));
    }

    #[test]
    fn health_cn002_undefined_variable() {
        let mut graph = ModelGraph::new();
        let elem = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("VarCheck")
            .with_prop("constraint", "x > 0");
        graph.add_element(elem);

        let diags = constraint_health_diagnostics(&graph);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code.as_deref(), Some("CN002"));
        assert!(diags[0].message.contains("variable 'x' has no value"));
    }

    #[test]
    fn health_cn003_violated() {
        let mut graph = ModelGraph::new();
        // Expression with only literals that evaluates to false
        let elem = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("AlwaysFalse")
            .with_prop("constraint", "1 > 2");
        graph.add_element(elem);

        let diags = constraint_health_diagnostics(&graph);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code.as_deref(), Some("CN003"));
        assert!(diags[0].message.contains("violated"));
    }

    #[test]
    fn health_cn004_non_boolean() {
        let mut graph = ModelGraph::new();
        // Expression that evaluates to a numeric value, not boolean
        let elem = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("NumericResult")
            .with_prop("constraint", "1 + 2");
        graph.add_element(elem);

        let diags = constraint_health_diagnostics(&graph);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code.as_deref(), Some("CN004"));
        assert!(diags[0].message.contains("expected boolean"));
    }

    #[test]
    fn health_no_diagnostic_for_passing_constraint() {
        let mut graph = ModelGraph::new();
        // Expression with only literals that evaluates to true
        let elem = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("AlwaysTrue")
            .with_prop("constraint", "1 < 2");
        graph.add_element(elem);

        let diags = constraint_health_diagnostics(&graph);
        assert!(
            diags.is_empty(),
            "passing constraint should produce no diagnostics"
        );
    }

    #[test]
    fn health_assert_constraint_usage_included() {
        let mut graph = ModelGraph::new();
        // AssertConstraintUsage with no expression
        let elem =
            Element::new_with_kind(ElementKind::AssertConstraintUsage).with_name("EmptyAssert");
        graph.add_element(elem);

        let diags = constraint_health_diagnostics(&graph);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code.as_deref(), Some("CN005"));
    }

    #[test]
    fn health_reference_form_constraint_usage_exempt_from_cn005() {
        let mut graph = ModelGraph::new();
        // A `require constraint : Def;` reference form: a bodyless ConstraintUsage
        // that owns a FeatureTyping delegating its predicate to the referenced
        // constraint (SysML §7.20). It must NOT get CN005 — its expression is the
        // type's, not absent.
        let usage = Element::new_with_kind(ElementKind::ConstraintUsage).with_name("refForm");
        let usage_id = graph.add_element(usage);
        let typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_owner(usage_id.clone())
            .with_prop("typedFeature", sysml_core::Value::Ref(usage_id.clone()))
            .with_prop("unresolved_type", "SomeConstraintDef");
        graph.add_element(typing);

        // A genuinely bodyless usage (no delegating typing/subsetting) still gets CN005.
        graph.add_element(Element::new_with_kind(ElementKind::ConstraintUsage).with_name("empty"));

        let diags = constraint_health_diagnostics(&graph);
        assert_eq!(
            diags.len(),
            1,
            "only the truly-bodyless usage is diagnosed, not the reference form: {diags:?}"
        );
        assert_eq!(diags[0].code.as_deref(), Some("CN005"));
        assert!(
            diags[0].message.contains("empty"),
            "CN005 names the bodyless usage, not the reference form: {}",
            diags[0].message
        );
    }

    #[test]
    fn health_mixed_constraints() {
        let mut graph = ModelGraph::new();

        // CN005: no expression
        graph.add_element(Element::new_with_kind(ElementKind::ConstraintUsage).with_name("NoExpr"));
        // CN001: bad expression
        graph.add_element(
            Element::new_with_kind(ElementKind::ConstraintUsage)
                .with_name("BadExpr")
                .with_prop("constraint", "!!!"),
        );
        // CN002: undefined var
        graph.add_element(
            Element::new_with_kind(ElementKind::ConstraintUsage)
                .with_name("UndefVar")
                .with_prop("constraint", "z > 0"),
        );
        // Pass: true literal
        graph.add_element(
            Element::new_with_kind(ElementKind::ConstraintUsage)
                .with_name("Ok")
                .with_prop("constraint", "1 < 2"),
        );

        let diags = constraint_health_diagnostics(&graph);
        // Should have 3 diagnostics (CN005, CN001, CN002) — the passing one is silent
        assert_eq!(diags.len(), 3);

        let codes: Vec<_> = diags.iter().filter_map(|d| d.code.as_deref()).collect();
        assert!(codes.contains(&"CN005"));
        assert!(codes.contains(&"CN001"));
        assert!(codes.contains(&"CN002"));
    }
}
