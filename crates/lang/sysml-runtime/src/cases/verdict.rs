//! Verdict data types: `VerdictKind`, `EvidenceRef`, `VerdictContext`, `Verdict`.

#[cfg(feature = "serde")]
use sysml_core::Value;

#[allow(unused_imports)]
use super::*;

// ---------------------------------------------------------------------------
// Verdict types (from VerificationCases.sysml)
// ---------------------------------------------------------------------------

/// The result of a verification case.
///
/// From the SysML v2 standard library:
/// ```sysml
/// enum def VerdictKind {
///     pass;
///     fail;
///     inconclusive;
///     error;
/// }
/// ```
// NOTE: serde uses the default derive, so the wire spelling is PascalCase
// (`"Pass"`), while `Display` — and therefore the session archive, the CLI
// and `engine/types.ts` — spell it lowercase. That split predates this type
// carrying constraint verdicts and is deliberately left alone here:
// collapsing it means re-blessing the `service-baseline` fixtures and
// retuning `sysml-cli::verify`, which is its own gated change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum VerdictKind {
    /// All requirements satisfied.
    Pass,
    /// One or more requirements not satisfied.
    Fail,
    /// Verification could not determine satisfaction.
    ///
    /// Also the [`Default`]: a verdict nobody has set is a determination
    /// nobody has made. Defaulting to `Pass` would assert an unperformed
    /// check succeeded, and to `Fail` an unperformed check found a
    /// violation; only `Inconclusive` claims nothing.
    #[default]
    Inconclusive,
    /// An error occurred during verification.
    Error,
}

impl VerdictKind {
    /// Returns true if this is a passing verdict.
    pub fn is_pass(self) -> bool {
        self == Self::Pass
    }

    /// Aggregate two verdicts (worst wins).
    ///
    /// Priority: Error > Fail > Inconclusive > Pass
    pub fn aggregate(self, other: Self) -> Self {
        match (self, other) {
            (Self::Error, _) | (_, Self::Error) => Self::Error,
            (Self::Fail, _) | (_, Self::Fail) => Self::Fail,
            (Self::Inconclusive, _) | (_, Self::Inconclusive) => Self::Inconclusive,
            (Self::Pass, Self::Pass) => Self::Pass,
        }
    }
}

impl std::fmt::Display for VerdictKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pass => write!(f, "pass"),
            Self::Fail => write!(f, "fail"),
            Self::Inconclusive => write!(f, "inconclusive"),
            Self::Error => write!(f, "error"),
        }
    }
}

// ---------------------------------------------------------------------------
// Universal Verdict struct (R1.3)
// ---------------------------------------------------------------------------
//
// Spec alignment: `VerdictKind` above (Pass/Fail/Inconclusive/Error) is the
// ground truth from `VerificationCases.sysml`. The `Verdict` struct here is an
// implementation-level extension: a single shape shared by every workflow
// (Run, Verify, Monte Carlo, Sweep, Trade Study) for carrying the evaluated
// value, expected value, margin, sensitivity, evidence pointer, and
// arbitrary metadata alongside the verdict.
//
// The design contract is documented in

/// A lightweight reference to the evidence that produced a verdict.
///
/// Points at a specific tick of a specific runtime session, optionally naming
/// the model element that caused the verdict (e.g. a requirement, constraint,
/// or threshold). Workflows in later rounds will fill this in so the UI can
/// deep-link from a verdict card back to the simulation state that generated it.
#[cfg(feature = "serde")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct EvidenceRef {
    /// Runtime session identifier (e.g. `"file://foo.sysml:MySm"`).
    pub session_id: String,
    /// Tick at which the verdict was evaluated.
    pub tick: u64,
    /// Optional model element identifier (requirement, constraint, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
}

/// Non-serde stub so downstream code can compile without the `serde` feature.
///
/// Any workflow that needs to carry evidence must enable the `serde` feature
/// (the sysml-service crate always does). The stub exists purely so the
/// re-export and the stub `Verdict` below compile in default builds.
#[cfg(not(feature = "serde"))]
#[derive(Debug, Clone)]
pub struct EvidenceRef {
    pub session_id: String,
    pub tick: u64,
    pub element_id: Option<String>,
}

/// Runtime context threaded through verdict construction sites so they can
/// attach an `EvidenceRef` pointing back at the session + tick that produced
/// the verdict.
///
/// Populated from the runtime `RuntimeSession` (session id + orchestrator
/// tick) at each verify call site. Construction sites that live in a static
/// (non-runtime) path — e.g. code-lens constraint evaluation without a live
/// session — leave it `None` and the resulting `Verdict.evidence` stays
/// `None` by design (see the doc comment on `evaluation::evaluate_constraints`
/// for the rationale).
#[derive(Debug, Clone)]
pub struct VerdictContext {
    /// Opaque runtime session identifier (UUID v4 from `execution::new_session_id`).
    pub session_id: String,
    /// Orchestrator tick at which the verdict is being emitted.
    pub tick: u64,
}

impl VerdictContext {
    /// Build a new verdict context from a live runtime session.
    pub fn new(session_id: impl Into<String>, tick: u64) -> Self {
        Self {
            session_id: session_id.into(),
            tick,
        }
    }

    /// Produce an [`EvidenceRef`] pointing at `element_id` within this session/tick.
    pub fn evidence_for(&self, element_id: Option<String>) -> EvidenceRef {
        EvidenceRef {
            session_id: self.session_id.clone(),
            tick: self.tick,
            element_id,
        }
    }
}

/// Universal verdict shape used by every workflow.
///
/// This is the single struct that every workflow emits — Run, Verify, Monte
/// Carlo, Sweep, Trade Study. Aggregators (count pass/fail, compute margins,
/// rank sensitivity) operate over `Vec<Verdict>` without caring which workflow
/// produced them.
///
/// | Workflow     | What produces a Verdict          | Evidence                 |
/// |--------------|----------------------------------|--------------------------|
/// | Run          | a constraint or assertion fires  | tick of the firing       |
/// | Verify       | each requirement check           | tick + requirement id    |
/// | Monte Carlo  | each scenario's rollup           | per-scenario session id  |
/// | Sweep        | each parameter combination       | per-variant session id   |
/// | Trade Study  | each alternative                 | per-alternative eval     |
///
/// ## Round 1 (this commit) — populated fields
///
/// - `verdict` — always populated
/// - `actual` — populated wherever the expression evaluator returns a value
///
/// ## Later rounds — deferred fields
///
/// - `expected` — Round 2 (needs requirement metadata extraction)
/// - `margin` — Round 2 (numeric constraints with threshold extraction)
/// - `sensitivity` — Round 5 (Sweep / sensitivity backend)
/// - `evidence` — Round 4 (session event stream)
/// - `metadata` — free-form; workflows push their own keys as needed
///
/// ## Serialization
///
/// Gated behind the `serde` feature. The `Value` payloads are `serde_json::Value`
/// so every workflow can round-trip the same shape across the service boundary
/// without needing a specific runtime `Value` variant.
#[cfg(feature = "serde")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Verdict {
    /// The four-valued verdict (spec: VerdictKind).
    pub verdict: VerdictKind,
    /// The value actually computed (e.g. the constraint's LHS or the observed
    /// metric). `None` when the workflow only cares about pass/fail semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<serde_json::Value>,
    /// The expected value or threshold the verdict was checked against.
    /// Populated by workflows that extract expected values from requirements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<serde_json::Value>,
    /// Numeric margin (actual − expected) when both are numeric.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin: Option<f64>,
    /// Sensitivity coefficients: input variable name → ∂verdict/∂input.
    /// Populated by Sweep / sensitivity workflows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<std::collections::HashMap<String, f64>>,
    /// Pointer back to the evidence (session + tick + element).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidenceRef>,
    /// Free-form metadata. Keys are workflow-specific (e.g. Monte Carlo may
    /// push `"scenario"`, Sweep may push `"variant_index"`, etc.).
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

/// Stub used when `serde` is disabled. Public so the re-export compiles, but
/// every field is `()` — no workflow can do anything meaningful without serde.
#[cfg(not(feature = "serde"))]
#[derive(Debug, Clone)]
pub struct Verdict {
    pub verdict: VerdictKind,
    pub actual: Option<()>,
    pub expected: Option<()>,
    pub margin: Option<f64>,
    pub sensitivity: Option<std::collections::HashMap<String, f64>>,
    pub evidence: Option<EvidenceRef>,
    pub metadata: std::collections::HashMap<String, ()>,
}

#[cfg(feature = "serde")]
impl Verdict {
    /// Construct a minimal verdict carrying only the four-valued outcome.
    ///
    /// Rounds 2+ populate the remaining fields as their workflows come online.
    pub fn new(verdict: VerdictKind) -> Self {
        Self {
            verdict,
            actual: None,
            expected: None,
            margin: None,
            sensitivity: None,
            evidence: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Attach an `actual` value.
    pub fn with_actual(mut self, actual: serde_json::Value) -> Self {
        self.actual = Some(actual);
        self
    }

    /// Attach an `expected` value.
    pub fn with_expected(mut self, expected: serde_json::Value) -> Self {
        self.expected = Some(expected);
        self
    }

    /// Attach an evidence pointer.
    pub fn with_evidence(mut self, evidence: EvidenceRef) -> Self {
        self.evidence = Some(evidence);
        self
    }

    /// Attach an evidence pointer derived from a live runtime session context.
    ///
    /// Convenience wrapper for the common runtime path where a caller has a
    /// [`VerdictContext`] (session + tick) and wants to bind the verdict to a
    /// specific SysML element — typically a `RequirementCheck`'s id or a
    /// `ConstraintUsage` element id.
    pub fn with_evidence_from_context(
        self,
        ctx: &VerdictContext,
        element_id: Option<String>,
    ) -> Self {
        self.with_evidence(ctx.evidence_for(element_id))
    }

    /// Convenience: build a verdict from a runtime `Value`, serializing it as
    /// `actual`. Unserializable variants drop to `None` silently — the verdict
    /// shape is always safe to emit, even for exotic values.
    pub fn from_value(verdict: VerdictKind, actual: Value) -> Self {
        let actual_json = value_to_json(&actual);
        Self::new(verdict).with_actual(actual_json)
    }

    /// Lift a [`RequirementResult`] into a universal [`Verdict`] and attach
    /// evidence from the runtime session context.
    ///
    /// Used by the verification runner once a session id + tick are known —
    /// downstream workflows (timeline, deep-link) need the evidence pointer
    /// so the UI can jump from a verdict card back to the exact session/tick.
    pub fn from_requirement_result_with_evidence(
        result: &RequirementResult,
        ctx: &VerdictContext,
    ) -> Self {
        let mut verdict: Self = result.into();
        verdict.evidence = Some(ctx.evidence_for(result.source_element_id.clone()));
        verdict
    }

    /// Lift a [`VerificationResult`] into a universal [`Verdict`] and attach
    /// evidence from the runtime session context.
    ///
    /// `case_element_id` is the SysML element id of the `VerificationCaseUsage`
    /// or `VerificationCaseDefinition` whose aggregate verdict this carries.
    pub fn from_verification_result_with_evidence(
        result: &VerificationResult,
        ctx: &VerdictContext,
        case_element_id: Option<String>,
    ) -> Self {
        let mut verdict: Self = result.into();
        verdict.evidence = Some(ctx.evidence_for(case_element_id));
        verdict
    }
}

#[cfg(not(feature = "serde"))]
impl Verdict {
    pub fn new(verdict: VerdictKind) -> Self {
        Self {
            verdict,
            actual: None,
            expected: None,
            margin: None,
            sensitivity: None,
            evidence: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Attach an evidence pointer derived from a runtime session context.
    ///
    /// Mirrors the serde-enabled variant so the API is the same across
    /// builds; non-serde builds still propagate evidence through the
    /// [`EvidenceRef`] stub.
    pub fn with_evidence_from_context(
        mut self,
        ctx: &VerdictContext,
        element_id: Option<String>,
    ) -> Self {
        self.evidence = Some(ctx.evidence_for(element_id));
        self
    }
}

/// Convert a runtime `Value` into a `serde_json::Value` for verdict payloads.
///
/// Round 1 handles the primitive variants. References and lists fall back to a
/// stringified form — later rounds can deepen this as needed.
#[cfg(feature = "serde")]
pub fn value_to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Int(i) => serde_json::Value::from(*i),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Null => serde_json::Value::Null,
        Value::List(items) => serde_json::Value::Array(items.iter().map(value_to_json).collect()),
        other => serde_json::Value::String(format!("{}", other)),
    }
}

#[cfg(feature = "serde")]
impl From<&RequirementResult> for Verdict {
    /// Lift a requirement check result into the universal Verdict shape.
    ///
    /// Carries the four-valued verdict and the explanatory message in
    /// `metadata["message"]`. `actual`/`expected` are left empty — Round 2
    /// will wire them to the constraint LHS/RHS once expected-value
    /// extraction from requirements lands.
    fn from(r: &RequirementResult) -> Self {
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "requirement_id".to_owned(),
            serde_json::Value::String(r.requirement_id.clone()),
        );
        if !r.message.is_empty() {
            metadata.insert(
                "message".to_owned(),
                serde_json::Value::String(r.message.clone()),
            );
        }
        Self {
            verdict: r.verdict,
            actual: None,
            expected: None,
            margin: None,
            sensitivity: None,
            evidence: None,
            metadata,
        }
    }
}

#[cfg(feature = "serde")]
impl From<&VerificationResult> for Verdict {
    /// Lift a verification case result into a single top-level Verdict.
    ///
    /// Individual requirement results should also be lifted separately (via
    /// `requirement_results.iter().map(Verdict::from)`) so the UI gets one
    /// verdict per requirement. This top-level verdict is the aggregate.
    fn from(r: &VerificationResult) -> Self {
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "requirement_count".to_owned(),
            serde_json::Value::from(r.requirement_results.len()),
        );
        let passed = r
            .requirement_results
            .iter()
            .filter(|rr| rr.verdict.is_pass())
            .count();
        metadata.insert("passed_count".to_owned(), serde_json::Value::from(passed));
        Self {
            verdict: r.verdict,
            actual: None,
            expected: None,
            margin: None,
            sensitivity: None,
            evidence: None,
            metadata,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn verdict_aggregation() {
        assert_eq!(
            VerdictKind::Pass.aggregate(VerdictKind::Pass),
            VerdictKind::Pass
        );
        assert_eq!(
            VerdictKind::Pass.aggregate(VerdictKind::Fail),
            VerdictKind::Fail
        );
        assert_eq!(
            VerdictKind::Fail.aggregate(VerdictKind::Inconclusive),
            VerdictKind::Fail
        );
        assert_eq!(
            VerdictKind::Inconclusive.aggregate(VerdictKind::Pass),
            VerdictKind::Inconclusive
        );
        assert_eq!(
            VerdictKind::Error.aggregate(VerdictKind::Pass),
            VerdictKind::Error
        );
    }

    #[test]
    fn verdict_display() {
        assert_eq!(format!("{}", VerdictKind::Pass), "pass");
        assert_eq!(format!("{}", VerdictKind::Fail), "fail");
        assert_eq!(format!("{}", VerdictKind::Inconclusive), "inconclusive");
        assert_eq!(format!("{}", VerdictKind::Error), "error");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn verdict_new_carries_only_kind() {
        let v = Verdict::new(VerdictKind::Pass);
        assert_eq!(v.verdict, VerdictKind::Pass);
        assert!(v.actual.is_none());
        assert!(v.expected.is_none());
        assert!(v.margin.is_none());
        assert!(v.sensitivity.is_none());
        assert!(v.evidence.is_none());
        assert!(v.metadata.is_empty());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn verdict_builder_attaches_actual() {
        let v = Verdict::new(VerdictKind::Fail).with_actual(serde_json::json!(42));
        assert_eq!(v.verdict, VerdictKind::Fail);
        assert_eq!(v.actual, Some(serde_json::json!(42)));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn verdict_roundtrips_json() {
        let original = Verdict::new(VerdictKind::Pass)
            .with_actual(serde_json::json!(3.14))
            .with_expected(serde_json::json!(3.0))
            .with_evidence(EvidenceRef {
                session_id: "file://foo.sysml:SM".into(),
                tick: 7,
                element_id: Some("req-1".into()),
            });

        let json = serde_json::to_string(&original).expect("serialize");
        let parsed: Verdict = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.verdict, VerdictKind::Pass);
        assert_eq!(parsed.actual, Some(serde_json::json!(3.14)));
        assert_eq!(parsed.expected, Some(serde_json::json!(3.0)));
        let ev = parsed.evidence.expect("evidence present");
        assert_eq!(ev.session_id, "file://foo.sysml:SM");
        assert_eq!(ev.tick, 7);
        assert_eq!(ev.element_id.as_deref(), Some("req-1"));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn verdict_skips_empty_optional_fields() {
        // Minimal verdict should not emit keys for None/empty fields.
        let v = Verdict::new(VerdictKind::Inconclusive);
        let json = serde_json::to_value(&v).unwrap();
        let obj = json.as_object().unwrap();
        assert!(obj.contains_key("verdict"));
        assert!(!obj.contains_key("actual"));
        assert!(!obj.contains_key("expected"));
        assert!(!obj.contains_key("margin"));
        assert!(!obj.contains_key("sensitivity"));
        assert!(!obj.contains_key("evidence"));
        assert!(!obj.contains_key("metadata"));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn verdict_from_requirement_result() {
        let rr = RequirementResult {
            requirement_id: "speed-limit".into(),
            source_element_id: None,
            verdict: VerdictKind::Fail,
            message: "constraint[0] failed".into(),
            assumptions_met: vec![],
            constraints_met: vec![false],
            subrequirement_results: vec![],
        };

        let verdict: Verdict = (&rr).into();
        assert_eq!(verdict.verdict, VerdictKind::Fail);
        assert_eq!(
            verdict.metadata.get("requirement_id"),
            Some(&serde_json::json!("speed-limit"))
        );
        assert_eq!(
            verdict.metadata.get("message"),
            Some(&serde_json::json!("constraint[0] failed"))
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn verdict_from_verification_result_aggregates() {
        let vr = VerificationResult {
            verdict: VerdictKind::Fail,
            requirement_results: vec![
                RequirementResult {
                    requirement_id: "r1".into(),
                    source_element_id: None,
                    verdict: VerdictKind::Pass,
                    message: String::new(),
                    assumptions_met: vec![],
                    constraints_met: vec![true],
                    subrequirement_results: vec![],
                },
                RequirementResult {
                    requirement_id: "r2".into(),
                    source_element_id: None,
                    verdict: VerdictKind::Fail,
                    message: String::new(),
                    assumptions_met: vec![],
                    constraints_met: vec![false],
                    subrequirement_results: vec![],
                },
            ],
            diagnostics: vec![],
        };

        let verdict: Verdict = (&vr).into();
        assert_eq!(verdict.verdict, VerdictKind::Fail);
        assert_eq!(
            verdict.metadata.get("requirement_count"),
            Some(&serde_json::json!(2))
        );
        assert_eq!(
            verdict.metadata.get("passed_count"),
            Some(&serde_json::json!(1))
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn verdict_kind_serializes_as_spec_variant() {
        // Spec ground truth: VerdictKind is the 4-valued enum from
        // VerificationCases.sysml. Ensure every variant round-trips so any
        // downstream consumer can rely on the name stability.
        for kind in [
            VerdictKind::Pass,
            VerdictKind::Fail,
            VerdictKind::Inconclusive,
            VerdictKind::Error,
        ] {
            let v = Verdict::new(kind);
            let json = serde_json::to_string(&v).unwrap();
            let parsed: Verdict = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed.verdict, kind);
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn verdict_context_produces_evidence_ref() {
        let ctx = VerdictContext::new("session-abc", 42);
        let ev = ctx.evidence_for(Some("req-speed".into()));
        assert_eq!(ev.session_id, "session-abc");
        assert_eq!(ev.tick, 42);
        assert_eq!(ev.element_id.as_deref(), Some("req-speed"));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn verdict_with_evidence_from_context_populates_fields() {
        let ctx = VerdictContext::new("sess-42", 7);
        let v = Verdict::new(VerdictKind::Fail)
            .with_evidence_from_context(&ctx, Some("element-99".into()));
        let ev = v.evidence.expect("evidence populated");
        assert_eq!(ev.session_id, "sess-42");
        assert_eq!(ev.tick, 7);
        assert_eq!(ev.element_id.as_deref(), Some("element-99"));
    }
}
