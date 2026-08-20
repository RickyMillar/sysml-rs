//! Requirement-check data types and value bindings.

use crate::expressions::ExprIR;
use sysml_core::Value;

#[allow(unused_imports)]
use super::*;

// ---------------------------------------------------------------------------
// Requirement check
// ---------------------------------------------------------------------------

/// A value binding that is resolved when a requirement is checked.
///
/// This keeps occurrence-specific bindings consumer/runtime neutral: compilation
/// records what the model binds, while the verifier resolves it against the
/// current `EvalContext` for this run. That lets simulation/analysis outputs
/// flow in through the context without making verification own those outputs.
#[derive(Debug, Clone)]
pub enum RequirementBinding {
    /// A concrete value known at compile/discovery time.
    Literal { name: String, value: Value },
    /// A feature path to evaluate in the check-time context, e.g.
    /// `massRun.massResult` or `subject.temperature`.
    FeaturePath { name: String, path: String },
    /// A feature path to evaluate at check time with a compile-time literal
    /// fallback for model-declared values. Dynamic context values still win.
    FeaturePathWithFallback {
        name: String,
        path: String,
        fallback: Value,
    },
    /// A compiled expression to evaluate in the check-time context.
    Expression { name: String, expr: ExprIR },
}

impl RequirementBinding {
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Literal { name, .. }
            | Self::FeaturePath { name, .. }
            | Self::FeaturePathWithFallback { name, .. }
            | Self::Expression { name, .. } => name,
        }
    }

    pub(crate) fn source_label(&self) -> String {
        match self {
            Self::Literal { .. } => "literal".into(),
            Self::FeaturePath { path, .. } | Self::FeaturePathWithFallback { path, .. } => {
                path.clone()
            }
            Self::Expression { .. } => "expression".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum BindingResolutionError {
    Undefined {
        binding: String,
        source: String,
        variable: String,
    },
    Evaluation {
        binding: String,
        source: String,
        message: String,
    },
}

/// A single requirement to be verified.
///
/// Requirements can contain sub-requirements that must also be satisfied.
/// This models the SysML v2 requirement hierarchy where a parent requirement
/// is satisfied only when all its children are also satisfied.
#[derive(Debug, Clone)]
pub struct RequirementCheck {
    /// Requirement identifier.
    pub id: String,
    /// Source model element id for this requirement, when compiled from a graph.
    pub source_element_id: Option<String>,
    /// Human-readable requirement text.
    pub text: Option<String>,
    /// Assumption expressions (if all true, constraints must hold).
    pub assumptions: Vec<ExprIR>,
    /// Constraint expressions (must all be true when assumptions hold).
    pub constraints: Vec<ExprIR>,
    /// Source model element ids for `constraints`, index-aligned when known.
    pub constraint_element_ids: Vec<Option<String>>,
    /// Errors encountered while compiling constraint expressions.
    ///
    /// When all constraints fail to compile and this is non-empty, the
    /// requirement produces an `Error` verdict instead of vacuously passing.
    pub compile_errors: Vec<String>,
    /// Nested sub-requirements that must all pass for this requirement to pass.
    pub subrequirements: Vec<RequirementCheck>,
    /// Per-occurrence value bindings for THIS verified requirement, overlaid onto
    /// the evaluation context before checking (overriding inherited/default values).
    ///
    /// Populated from a `verify requirement R { attribute x = v; }` clause's
    /// redefinition members. Per spec the `=` is a bound `FeatureValue`,
    /// equivalent to a BindingConnector forcing value equality
    /// (VerificationCases.sysml:21-27; SysML-vocab.ttl:423-425). Occurrence-scoped:
    /// each verified requirement carries its own overlay, so same-named attributes
    /// across requirements never collide (a flat shared context would clobber them).
    pub bindings: Vec<(String, Value)>,
    /// Lazy/evaluable per-occurrence bindings. These are resolved against the
    /// check-time context after `bindings`, preserving the same occurrence scope
    /// while allowing feature references and computed values to consume dynamic
    /// run outputs.
    pub binding_specs: Vec<RequirementBinding>,
}

/// Result of checking a single requirement.
#[derive(Debug, Clone)]
pub struct RequirementResult {
    /// The requirement that was checked.
    pub requirement_id: String,
    /// Source model element id for this requirement, threaded from
    /// [`RequirementCheck::source_element_id`] so verdicts/evidence can deep-link
    /// to the real element rather than the (non-unique) requirement name.
    pub source_element_id: Option<String>,
    /// Whether the requirement was satisfied.
    pub verdict: VerdictKind,
    /// Detailed explanation.
    pub message: String,
    /// Which assumptions held.
    pub assumptions_met: Vec<bool>,
    /// Which constraints were satisfied.
    pub constraints_met: Vec<bool>,
    /// Results of nested sub-requirements.
    pub subrequirement_results: Vec<RequirementResult>,
}

