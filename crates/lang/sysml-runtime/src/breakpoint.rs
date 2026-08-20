//! Breakpoint primitives for runtime session debugging.
//!
//! A `Breakpoint` describes a hit condition that the runtime session
//! evaluates at step boundaries. When a breakpoint matches, the session
//! pauses and records which breakpoint fired so clients can introspect
//! state before resuming.
//!
//! Breakpoints are not part of the SysML v2 spec — they are a sysml-rs
//! addition layered on top of the runtime. The enum shape is intentionally
//! small for R1.2: a handful of common hit types, a compare operator for
//! threshold crossings, and an opaque UUID id. Richer conditionals
//! (hit counts, log points, etc.) are deferred to later rounds.
//!
//! phases 4-5 for the design background.

/// Opaque identifier for a registered breakpoint. Currently a UUIDv4 string.
pub type BreakpointId = String;

/// A comparison operator used by `Breakpoint::ThresholdCrossing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum CompareOp {
    /// Less than (`x < value`).
    Lt,
    /// Less than or equal (`x <= value`).
    Le,
    /// Greater than (`x > value`).
    Gt,
    /// Greater than or equal (`x >= value`).
    Ge,
    /// Equal (`x == value`).
    Eq,
    /// Not equal (`x != value`).
    Ne,
}

impl CompareOp {
    /// Apply this operator to a pair of f64s.
    pub fn apply(self, lhs: f64, rhs: f64) -> bool {
        match self {
            CompareOp::Lt => lhs < rhs,
            CompareOp::Le => lhs <= rhs,
            CompareOp::Gt => lhs > rhs,
            CompareOp::Ge => lhs >= rhs,
            CompareOp::Eq => (lhs - rhs).abs() < f64::EPSILON,
            CompareOp::Ne => (lhs - rhs).abs() >= f64::EPSILON,
        }
    }
}

/// Shared field set for the `ThresholdCrossing` / `Conditional` breakpoint
/// variants (BP4 collapse, core-steward ruling 2026-07-14).
///
/// The two variants used to carry near-duplicate field sets — a
/// `ThresholdCrossing { variable, op, value, debounce_ticks }` with no
/// owning element, and a `Conditional { target, variable, op, value,
/// enabled, label }` explicitly scoped to an element. `Conditional`
/// subsumes `ThresholdCrossing`: both variants now wrap this one struct,
/// with `target` optional (absent ⇒ unscoped, matching the historical
/// `ThresholdCrossing` shape) and `enabled`/`debounce_ticks` shared by
/// both. `element_id()` returns `None` when `target` is `None`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CompareBreakpoint {
    /// Name of the variable to watch — in the shared `EvalContext` /
    /// tick snapshot's `variables` map.
    pub variable: String,
    /// Comparison operator.
    pub op: CompareOp,
    /// Value the variable is compared against.
    pub value: f64,
    /// Debounce window (in ticks) — once this breakpoint fires, it is
    /// suppressed for `debounce_ticks` ticks before re-arming. Prevents
    /// firing every tick while the condition stays true. Default `0`
    /// (no debouncing) so existing callers that serialized without
    /// this field continue to round-trip.
    #[cfg_attr(feature = "serde", serde(default))]
    pub debounce_ticks: u32,
    /// Whether the breakpoint is armed. When `false`, the evaluator skips
    /// this entry. Defaults to `true` if absent in the JSON — matches the
    /// historical `ThresholdCrossing` shape, which had no `enabled` field
    /// at all and was always considered armed.
    #[cfg_attr(feature = "serde", serde(default = "default_true"))]
    pub enabled: bool,
    /// Element this condition is attached to (usually the owning part or
    /// state). `None` for the historical unscoped `ThresholdCrossing`
    /// form — free-form stringly-typed id for transport when present.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub target: Option<String>,
    /// Optional user-facing label shown in the UI. `None` when the UI
    /// should compute a default from `variable op value`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub label: Option<String>,
}

/// A breakpoint condition registered against a running session.
///
/// Hit detection is performed by the owning session at step boundaries
/// (state-entry, transition-fire, action-invoke, constraint-check,
/// variable-assignment). When a breakpoint matches, the session pauses
/// and records the firing id.
///
/// The `kind` tag is serialized in kebab-case (`state-entry`,
/// `transition-fire`, `action-invoke`, `constraint-violation`,
/// `threshold-crossing`, `conditional`) so the frontend `SessionControl`
/// API can consume them directly. `threshold-crossing` and `conditional`
/// are two serde tags over the SAME [`CompareBreakpoint`] field set (BP4
/// collapse) — kept as two variants (rather than one renamed variant with
/// a serde alias) so persisted/archived payloads and the current frontend
/// wire both keep deserializing under their original tag name with no
/// forced migration.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "kebab-case"))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum Breakpoint {
    /// Pause when a state with this element id is entered.
    StateEntry {
        /// Element id of the target state (stringly-typed for transport simplicity).
        element_id: String,
    },
    /// Pause when a transition with this element id fires.
    TransitionFire {
        /// Element id of the target transition.
        element_id: String,
    },
    /// Pause when an action node with this element id is invoked.
    ActionInvoke {
        /// Element id of the target action node.
        element_id: String,
    },
    /// Pause when a constraint with this element id evaluates to false.
    ConstraintViolation {
        /// Element id of the target constraint.
        element_id: String,
    },
    /// Pause when a variable's numeric value crosses the given threshold.
    ///
    /// Historically unscoped (no owning element); `element_id()` returns
    /// `None` unless `target` was explicitly set. See [`CompareBreakpoint`].
    ThresholdCrossing(CompareBreakpoint),
    /// Pause when a variable compares against a value per `op`.
    ///
    /// Historically scoped to an owning element (`target`) with a
    /// user-editable `enabled` flag + optional label. See
    /// [`CompareBreakpoint`].
    Conditional(CompareBreakpoint),
}

#[cfg(feature = "serde")]
fn default_true() -> bool {
    true
}

impl Breakpoint {
    /// Helper: construct a state-entry breakpoint.
    pub fn state_entry(element_id: impl Into<String>) -> Self {
        Breakpoint::StateEntry {
            element_id: element_id.into(),
        }
    }

    /// Helper: construct a transition-fire breakpoint.
    pub fn transition_fire(element_id: impl Into<String>) -> Self {
        Breakpoint::TransitionFire {
            element_id: element_id.into(),
        }
    }

    /// Helper: construct an action-invoke breakpoint.
    pub fn action_invoke(element_id: impl Into<String>) -> Self {
        Breakpoint::ActionInvoke {
            element_id: element_id.into(),
        }
    }

    /// Helper: construct a constraint-violation breakpoint.
    pub fn constraint_violation(element_id: impl Into<String>) -> Self {
        Breakpoint::ConstraintViolation {
            element_id: element_id.into(),
        }
    }

    /// Helper: construct a threshold-crossing breakpoint (no debounce, no
    /// owning element).
    pub fn threshold_crossing(variable: impl Into<String>, op: CompareOp, value: f64) -> Self {
        Breakpoint::ThresholdCrossing(CompareBreakpoint {
            variable: variable.into(),
            op,
            value,
            debounce_ticks: 0,
            enabled: true,
            target: None,
            label: None,
        })
    }

    /// Helper: construct a threshold-crossing breakpoint with a debounce
    /// window (ticks suppressed after a firing).
    pub fn threshold_crossing_with_debounce(
        variable: impl Into<String>,
        op: CompareOp,
        value: f64,
        debounce_ticks: u32,
    ) -> Self {
        Breakpoint::ThresholdCrossing(CompareBreakpoint {
            variable: variable.into(),
            op,
            value,
            debounce_ticks,
            enabled: true,
            target: None,
            label: None,
        })
    }

    /// Helper: construct a conditional breakpoint (enabled, no label, no debounce).
    pub fn conditional(
        target: impl Into<String>,
        variable: impl Into<String>,
        op: CompareOp,
        value: f64,
    ) -> Self {
        Breakpoint::Conditional(CompareBreakpoint {
            variable: variable.into(),
            op,
            value,
            debounce_ticks: 0,
            enabled: true,
            target: Some(target.into()),
            label: None,
        })
    }

    /// Returns the element id associated with this breakpoint, if any.
    ///
    /// `ThresholdCrossing` / `Conditional` return their shared `target`
    /// field (`None` when unscoped — the historical `ThresholdCrossing`
    /// shape).
    pub fn element_id(&self) -> Option<&str> {
        match self {
            Breakpoint::StateEntry { element_id }
            | Breakpoint::TransitionFire { element_id }
            | Breakpoint::ActionInvoke { element_id }
            | Breakpoint::ConstraintViolation { element_id } => Some(element_id.as_str()),
            Breakpoint::ThresholdCrossing(f) | Breakpoint::Conditional(f) => f.target.as_deref(),
        }
    }

    /// Test whether this breakpoint matches a state-entry event for the
    /// given element id or state name.
    pub fn matches_state_entry(&self, id_or_name: &str) -> bool {
        matches!(
            self,
            Breakpoint::StateEntry { element_id } if element_id == id_or_name
        )
    }

    /// Test whether this breakpoint matches a transition-fire event.
    pub fn matches_transition_fire(&self, id_or_name: &str) -> bool {
        matches!(
            self,
            Breakpoint::TransitionFire { element_id } if element_id == id_or_name
        )
    }

    /// Test whether this breakpoint matches an action-invoke event.
    pub fn matches_action_invoke(&self, id_or_name: &str) -> bool {
        matches!(
            self,
            Breakpoint::ActionInvoke { element_id } if element_id == id_or_name
        )
    }

    /// Test whether this breakpoint matches a constraint-violation event.
    pub fn matches_constraint_violation(&self, id_or_name: &str) -> bool {
        matches!(
            self,
            Breakpoint::ConstraintViolation { element_id } if element_id == id_or_name
        )
    }

    /// Test whether this breakpoint (`ThresholdCrossing` or `Conditional`
    /// — the two [`CompareBreakpoint`]-backed variants) matches a
    /// variable's current numeric value.
    ///
    /// Returns `false` for the four event-based variants
    /// (`StateEntry`/`TransitionFire`/`ActionInvoke`/`ConstraintViolation`),
    /// for a variable-name mismatch, or when `enabled` is `false`.
    /// Historical `ThresholdCrossing` payloads are always `enabled: true`
    /// (default), so this is a drop-in replacement for the old
    /// `matches_threshold` on that variant.
    pub fn matches(&self, variable: &str, current: f64) -> bool {
        match self {
            Breakpoint::ThresholdCrossing(f) | Breakpoint::Conditional(f) => {
                f.enabled && f.variable == variable && f.op.apply(current, f.value)
            }
            _ => false,
        }
    }
}

/// Generate a fresh opaque breakpoint id.
pub fn new_breakpoint_id() -> BreakpointId {
    // Deterministic-ish random id — uses the system randomness underlying
    // `std::time` so we don't pull in another crate for this alone.
    //
    // NOTE: `uuid` is available in the service crate but not here (runtime
    // Cargo.toml is kept lean). A 128-bit hex derived from system time +
    // process randomness is sufficient for breakpoint identification
    // within a single session.
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    // Fold into a UUIDv4-like representation (36 chars, hyphenated).
    let a = (now >> 32) as u32;
    let b = (now & 0xFFFF_FFFF) as u32;
    let c = (seq >> 16) as u16;
    let d = ((seq & 0xFFFF) as u16) | 0x4000; // version-4-ish nibble
    let e = now.wrapping_mul(seq.wrapping_add(1));
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        a,
        (b >> 16) as u16,
        c,
        d,
        e & 0xFFFF_FFFF_FFFF,
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn compare_op_apply() {
        assert!(CompareOp::Lt.apply(1.0, 2.0));
        assert!(!CompareOp::Lt.apply(2.0, 2.0));
        assert!(CompareOp::Le.apply(2.0, 2.0));
        assert!(CompareOp::Gt.apply(3.0, 2.0));
        assert!(CompareOp::Ge.apply(2.0, 2.0));
        assert!(CompareOp::Eq.apply(2.0, 2.0));
        assert!(CompareOp::Ne.apply(1.0, 2.0));
        assert!(!CompareOp::Ne.apply(2.0, 2.0));
    }

    #[test]
    fn breakpoint_helpers_and_matchers() {
        let bp = Breakpoint::state_entry("state-42");
        assert_eq!(bp.element_id(), Some("state-42"));
        assert!(bp.matches_state_entry("state-42"));
        assert!(!bp.matches_state_entry("state-99"));
        assert!(!bp.matches_transition_fire("state-42"));

        let bp = Breakpoint::transition_fire("t1");
        assert!(bp.matches_transition_fire("t1"));
        assert!(!bp.matches_state_entry("t1"));

        let bp = Breakpoint::action_invoke("a1");
        assert!(bp.matches_action_invoke("a1"));

        let bp = Breakpoint::constraint_violation("c1");
        assert!(bp.matches_constraint_violation("c1"));

        // (b) a threshold-crossing (numeric, rising) still fires.
        let bp = Breakpoint::threshold_crossing("voltage", CompareOp::Gt, 3.3);
        assert_eq!(bp.element_id(), None);
        assert!(bp.matches("voltage", 3.4));
        assert!(!bp.matches("voltage", 3.2));
        assert!(!bp.matches("current", 3.4));
    }

    #[test]
    fn new_breakpoint_id_is_unique() {
        let a = new_breakpoint_id();
        let b = new_breakpoint_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36, "UUID-like id should be 36 chars, got `{a}`");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn breakpoint_serde_round_trip_state_entry() {
        let bp = Breakpoint::state_entry("state-123");
        let json = serde_json::to_string(&bp).unwrap();
        assert!(json.contains("\"kind\":\"state-entry\""), "got: {json}");
        assert!(json.contains("\"element_id\":\"state-123\""), "got: {json}");
        let back: Breakpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(back, bp);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn breakpoint_serde_round_trip_threshold_crossing() {
        let bp = Breakpoint::threshold_crossing("v_busbar", CompareOp::Ge, 12.5);
        let json = serde_json::to_string(&bp).unwrap();
        assert!(
            json.contains("\"kind\":\"threshold-crossing\""),
            "got: {json}"
        );
        assert!(json.contains("\"variable\":\"v_busbar\""), "got: {json}");
        assert!(json.contains("\"op\":\"ge\""), "got: {json}");
        let back: Breakpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(back, bp);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn breakpoint_serde_all_variants_round_trip() {
        let variants = [
            Breakpoint::state_entry("s1"),
            Breakpoint::transition_fire("t1"),
            Breakpoint::action_invoke("a1"),
            Breakpoint::constraint_violation("c1"),
            Breakpoint::threshold_crossing("x", CompareOp::Lt, -1.0),
            Breakpoint::conditional("circuit1", "v_bus", CompareOp::Gt, 12.0),
        ];
        for bp in variants {
            let json = serde_json::to_string(&bp).unwrap();
            let back: Breakpoint = serde_json::from_str(&json).unwrap();
            assert_eq!(back, bp);
        }
    }

    #[test]
    fn conditional_breakpoint_constructors_and_matchers() {
        let bp = Breakpoint::conditional("circuit1", "voltage", CompareOp::Gt, 12.0);
        assert_eq!(bp.element_id(), Some("circuit1"));
        // Fires when the condition is true
        assert!(bp.matches("voltage", 13.0));
        // Does not fire when the condition is false
        assert!(!bp.matches("voltage", 10.0));
        // Does not fire for a different variable
        assert!(!bp.matches("current", 13.0));

        // Disabled conditional should never match.
        let disabled = Breakpoint::Conditional(CompareBreakpoint {
            variable: "v".to_string(),
            op: CompareOp::Gt,
            value: 0.0,
            debounce_ticks: 0,
            enabled: false,
            target: Some("x".to_string()),
            label: None,
        });
        assert!(!disabled.matches("v", 100.0));
    }

    #[test]
    fn conditional_handles_numeric_edge_cases() {
        let bp = Breakpoint::conditional("p", "x", CompareOp::Eq, 0.0);
        // Equality against zero.
        assert!(bp.matches("x", 0.0));
        // Negative comparison.
        let neg = Breakpoint::conditional("p", "x", CompareOp::Lt, 0.0);
        assert!(neg.matches("x", -1.0));
        assert!(!neg.matches("x", 0.0));
        // NaN never satisfies any comparison — IEEE 754 semantics.
        for op in [
            CompareOp::Lt,
            CompareOp::Le,
            CompareOp::Gt,
            CompareOp::Ge,
            CompareOp::Eq,
        ] {
            let bp = Breakpoint::conditional("p", "x", op, 0.0);
            assert!(
                !bp.matches("x", f64::NAN),
                "NaN should not match {op:?}"
            );
        }
        // Ne against NaN — epsilon check treats NaN as "not equal" (|NaN - rhs| >= eps is false)
        // so matches_conditional returns false as well. Assert for parity.
        let ne = Breakpoint::conditional("p", "x", CompareOp::Ne, 0.0);
        assert!(!ne.matches("x", f64::NAN));
    }

    /// (a) a bool context var flipping to `true` fires an `op:Eq value:1.0`
    /// breakpoint on that tick (BP3 — bool→f64 coercion at the compare
    /// site, `1.0 Eq 1.0`).
    #[test]
    fn bool_flip_fires_eq_one_breakpoint() {
        let bp = Breakpoint::conditional("circuit1", "tripped", CompareOp::Eq, 1.0);
        // Bool coercion happens at the snapshot-lookup call site
        // (`sysml_runtime::snapshot_view::value_to_scalar`), not inside
        // `Breakpoint` — this test exercises the compare itself once the
        // bool has already been coerced to `1.0`/`0.0`, matching the
        // convention in `constraints.rs::value_to_scalar` /
        // `snapshot_view.rs::value_to_scalar`.
        assert!(bp.matches("tripped", 1.0), "true -> 1.0 should fire Eq 1.0");
        assert!(!bp.matches("tripped", 0.0), "false -> 0.0 should not fire Eq 1.0");
    }

    #[test]
    fn threshold_crossing_default_debounce_is_zero() {
        let bp = Breakpoint::threshold_crossing("x", CompareOp::Gt, 0.0);
        match bp {
            Breakpoint::ThresholdCrossing(f) => assert_eq!(f.debounce_ticks, 0),
            _ => panic!("expected ThresholdCrossing variant"),
        }
    }

    #[test]
    fn threshold_crossing_with_debounce_constructor() {
        let bp = Breakpoint::threshold_crossing_with_debounce("x", CompareOp::Gt, 1.0, 5);
        match bp {
            Breakpoint::ThresholdCrossing(f) => assert_eq!(f.debounce_ticks, 5),
            _ => panic!("expected ThresholdCrossing variant"),
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn threshold_crossing_missing_debounce_defaults_to_zero() {
        // Old JSON (pre-R4.4) has no `debounce_ticks` — must still deserialize.
        let legacy = r#"{"kind":"threshold-crossing","variable":"v","op":"ge","value":12.5}"#;
        let bp: Breakpoint = serde_json::from_str(legacy).unwrap();
        match bp {
            Breakpoint::ThresholdCrossing(f) => {
                assert_eq!(f.variable, "v");
                assert_eq!(f.op, CompareOp::Ge);
                assert!((f.value - 12.5).abs() < f64::EPSILON);
                assert_eq!(f.debounce_ticks, 0);
                assert!(f.enabled, "legacy threshold-crossing has no `enabled` field — must default true");
                assert_eq!(f.target, None);
                assert_eq!(f.label, None);
            }
            _ => panic!("expected ThresholdCrossing variant"),
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn conditional_breakpoint_serde_round_trip() {
        let bp = Breakpoint::conditional("circuit1", "voltage", CompareOp::Gt, 12.0);
        let json = serde_json::to_string(&bp).unwrap();
        assert!(json.contains("\"kind\":\"conditional\""), "got: {json}");
        assert!(json.contains("\"target\":\"circuit1\""), "got: {json}");
        assert!(json.contains("\"variable\":\"voltage\""), "got: {json}");
        assert!(json.contains("\"op\":\"gt\""), "got: {json}");
        assert!(json.contains("\"value\":12.0"), "got: {json}");
        let back: Breakpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(back, bp);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn conditional_breakpoint_enabled_defaults_true_when_missing() {
        // Abbreviated JSON (no `enabled`) should still enable the bp.
        let json = r#"{"kind":"conditional","target":"p","variable":"x","op":"lt","value":1.0}"#;
        let bp: Breakpoint = serde_json::from_str(json).unwrap();
        match bp {
            Breakpoint::Conditional(f) => assert!(f.enabled),
            _ => panic!("expected Conditional variant"),
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn threshold_crossing_serde_round_trip_with_debounce() {
        let bp = Breakpoint::threshold_crossing_with_debounce("v", CompareOp::Ge, 12.5, 3);
        let json = serde_json::to_string(&bp).unwrap();
        assert!(json.contains("\"debounce_ticks\":3"), "got: {json}");
        let back: Breakpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(back, bp);
    }

    /// (c) both serde `kind` tags deserialize into the unified struct with
    /// defaults: the frontend's abbreviated `threshold-crossing` payload
    /// (no target/enabled/label) and its fuller `conditional` payload
    /// (with target/enabled/label) both land on `CompareBreakpoint` with
    /// the missing fields defaulted.
    #[cfg(feature = "serde")]
    #[test]
    fn both_kind_tags_deserialize_into_unified_struct_with_defaults() {
        let threshold_json =
            r#"{"kind":"threshold-crossing","variable":"v_bus","op":"ge","value":12.5}"#;
        let bp: Breakpoint = serde_json::from_str(threshold_json).unwrap();
        match bp {
            Breakpoint::ThresholdCrossing(f) => {
                assert_eq!(f.variable, "v_bus");
                assert_eq!(f.op, CompareOp::Ge);
                assert!((f.value - 12.5).abs() < f64::EPSILON);
                assert_eq!(f.debounce_ticks, 0, "missing debounce_ticks defaults to 0");
                assert!(f.enabled, "missing enabled defaults to true");
                assert_eq!(f.target, None, "missing target defaults to None");
                assert_eq!(f.label, None, "missing label defaults to None");
            }
            other => panic!("expected ThresholdCrossing, got {other:?}"),
        }

        let conditional_json = r#"{"kind":"conditional","target":"circuit1","variable":"voltage","op":"gt","value":12.0,"enabled":true,"label":"overvoltage"}"#;
        let bp: Breakpoint = serde_json::from_str(conditional_json).unwrap();
        match bp {
            Breakpoint::Conditional(f) => {
                assert_eq!(f.variable, "voltage");
                assert_eq!(f.op, CompareOp::Gt);
                assert!((f.value - 12.0).abs() < f64::EPSILON);
                assert_eq!(f.debounce_ticks, 0, "missing debounce_ticks defaults to 0");
                assert!(f.enabled);
                assert_eq!(f.target.as_deref(), Some("circuit1"));
                assert_eq!(f.label.as_deref(), Some("overvoltage"));
            }
            other => panic!("expected Conditional, got {other:?}"),
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn compare_op_serde_kebab_case() {
        for (op, expected) in [
            (CompareOp::Lt, "\"lt\""),
            (CompareOp::Le, "\"le\""),
            (CompareOp::Gt, "\"gt\""),
            (CompareOp::Ge, "\"ge\""),
            (CompareOp::Eq, "\"eq\""),
            (CompareOp::Ne, "\"ne\""),
        ] {
            let json = serde_json::to_string(&op).unwrap();
            assert_eq!(json, expected, "op {op:?} serialization");
            let back: CompareOp = serde_json::from_str(&json).unwrap();
            assert_eq!(back, op);
        }
    }
}
