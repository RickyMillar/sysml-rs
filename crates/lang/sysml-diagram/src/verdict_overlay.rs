//! Per-run **verdict/solver** overlay-delta (requirements/parametric Phase 2).
//!
//! A renderer-agnostic, `ElementId`-joined description of how a diagram should be
//! decorated with the latest **constraint solver verdict** for one execution
//! run: which constraint usages passed / failed and, when unambiguous, the
//! solved scalar value behind that verdict. It is *not* pixels — the renderer
//! owns presentation (✓/✗ glyph, badge color). The overlay only says *what the
//! verdict is* for this run's latest snapshot.
//!
//! ## Why this is NOT on the salsa `ViewModel`
//!
//! `tokens` / `text_map` / `interactions` are each a **pure function of the
//! graph**, so each rides a graph-keyed salsa sidecar. The verdict overlay is
//! **session state** — a function of a live `RuntimeSession`'s latest
//! [`ExecutionSnapshot::constraint_results`] — and it changes every run/tick. It
//! is not salsa-cacheable on the graph. So it is built and delivered at the
//! **service layer** as a *separate artifact* via `sysml.diagram.verdict_overlay`;
//! there is deliberately **no `overlays` field on `ViewModel`** (it would be
//! permanently `None` on the salsa path — steward ruling 2026-06-25, option A).
//!
//! This is exactly why the retired `Parametric` peer generator was wrong: it
//! baked solver pass/fail badges + solved values into the graph-pure,
//! salsa-cached scene. Session-derived verdict state lives here, per run.
//!
//! ## Not to be confused with [`crate::ir::overlays`]
//!
//! `crate::ir::overlays` (the `DiagramOverlay` trait, parametric/requirement
//! projection) is a **structural build-time** overlay — a different concept. This
//! module is the **per-run verdict** sidecar. They never mix.
//!
//! ## The `ElementId` join (identity-first)
//!
//! Each [`ConstraintEvalResult`] carries the constraint usage's
//! [`ConstraintEvalResult::element_id`] (forwarded from `ConstraintIR.owner_id`).
//! That id — never a reconstructed name string — is the join key, identical to
//! the string used as the scene node id (so the renderer joins directly to
//! `DiagramNode::element_id`). A result with no `element_id` (legacy IR path)
//! cannot be joined and is skipped; a result whose id is not present in the
//! current scene is skipped (the overlay stays sparse and scene-scoped, matching
//! how `sim_overlay`'s value badges only attach to scene nodes).

use std::collections::HashMap;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use sysml_runtime::orchestrator::ExecutionSnapshot;
use sysml_runtime::VerdictKind;

use crate::ir::types::{DiagramChild, DiagramIR, DiagramNode};

/// The per-run verdict overlay for a diagram scene.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct VerdictOverlay {
    /// Simulation tick this overlay reflects.
    pub tick: u64,
    /// Simulation time in milliseconds.
    pub time_ms: f64,
    /// Tick at which the session's stored verification verdicts were computed
    /// (`sysml.sessions.verify`), when any were supplied. The session may have
    /// advanced since (`tick > verified_at_tick`); the verdicts are then
    /// STALE-but-labeled — deliberately kept rather than silently dropped
    /// (steward ruling 2026-07-14: a vanishing pill reads as "nothing
    /// verified", which hides information; the frontend diffs the two ticks to
    /// badge staleness instead).
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub verified_at_tick: Option<u64>,
    /// Per-element verdict deltas. **Sparse** — only constraint elements that
    /// produced a verdict joinable to this scene.
    ///
    /// Key = [`ElementId::to_string`](sysml_core::ElementId::to_string) (the same
    /// string used as the scene node id, so the renderer joins directly to
    /// `DiagramNode::element_id`). **Never a name string** — every key derives
    /// from an `ElementId`.
    pub elements: HashMap<String, ElementVerdict>,
}

/// A session's stored verification outcome (`sysml.sessions.verify`) in the
/// resolved per-element shape this builder joins: one row per requirement (and
/// per requirement-constraint) that carried a real `ElementId`.
///
/// The producer is `sessions_verify` ONLY — the one `VerificationRunner` path
/// against the live session context (steward, 2026-07-14). Routing any other
/// verdict source (e.g. the demoted `verify_with_simulation`) through this
/// type would reopen the second-verdict-path problem Inc2b exists to close.
#[derive(Debug, Clone, PartialEq)]
pub struct VerificationVerdicts {
    /// Session tick when the verification ran (the staleness anchor).
    pub verified_at_tick: u64,
    /// `(element, verdict, value)` rows — requirement elements carry the
    /// requirement verdict; constraint elements their individual satisfaction.
    /// Hard `ElementId`s, parsed once at the producer (never re-stringified
    /// names).
    pub verdicts: Vec<(sysml_core::ElementId, VerdictKind, Option<f64>)>,
}

/// A single element's verdict delta for this run. All fields are `Option` —
/// absent means "no information to report on this facet".
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ElementVerdict {
    /// The solver verdict for this constraint, forwarded verbatim from
    /// [`ConstraintEvalResult::verdict`]. All four [`VerdictKind`] values
    /// reach here: a constraint the run could not decide (unbound
    /// parameter, non-boolean result, evaluator error) badges as
    /// `Inconclusive` rather than being flattened into `Fail`.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub verdict: Option<VerdictKind>,
    /// The solved scalar value behind the verdict, when it is unambiguous.
    ///
    /// A constraint references zero or more live operands. We only badge a value
    /// when the constraint has **exactly one** referenced operand (the single
    /// value the verdict is "about"); with zero or several operands there is no
    /// single value to show, so this is `None` (we do not guess / sum / pick).
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub value: Option<f64>,
}

impl ElementVerdict {
    fn is_empty(&self) -> bool {
        self.verdict.is_none() && self.value.is_none()
    }
}

/// Build the per-run [`VerdictOverlay`] for `scene` from a session's latest
/// `snapshot` plus its stored verification outcome, if any.
///
/// Pure and deterministic. The builder lives here (not the service layer)
/// because the join is against the **scene**, and `sysml-diagram` already
/// depends on `sysml-runtime` for the snapshot type (principle #6, lowest
/// reasonable crate).
///
/// Two producers merge into the one `elements` map:
/// 1. Per-tick solver state: each
///    [`ConstraintEvalResult`](sysml_runtime::orchestrator::ConstraintEvalResult)
///    with an `element_id` that resolves to a node in `scene`.
/// 2. The session's latest `sysml.sessions.verify` outcome
///    ([`VerificationVerdicts`]), applied second: on the (rare but possible —
///    a requirement's checks ARE constraint usages per Requirements.sysml)
///    collision, the **verification verdict wins**: it is the richer,
///    user-initiated judgment.
///
/// Rows with no `element_id`, or whose id is not in the scene, are skipped
/// (sparse, scene-scoped, no fabricated placement).
pub fn build_verdict_overlay(
    scene: &DiagramIR,
    snapshot: &ExecutionSnapshot,
    verification: Option<&VerificationVerdicts>,
) -> VerdictOverlay {
    let mut elements: HashMap<String, ElementVerdict> = HashMap::new();

    for result in &snapshot.constraint_results {
        // Identity-first: no element id means no honest join target — skip
        // rather than fabricate one from the (collision-prone) name.
        let Some(element_id) = result.element_id.as_ref() else {
            continue;
        };
        let key = element_id.to_string();

        // Scene-scoped: only decorate constraint nodes actually present in this
        // view's scene (mirrors sim_overlay value badges, which only attach to
        // scene nodes). A verdict for an off-scene element has nothing to badge.
        if find_node(&scene.nodes, &key).is_none() {
            continue;
        }

        let verdict = Some(result.verdict);

        // Unambiguous solved value: exactly one referenced operand.
        let value = if result.operands.len() == 1 {
            result.operands.values().next().copied()
        } else {
            None
        };

        elements.insert(key, ElementVerdict { verdict, value });
    }

    // Stored verification verdicts, second: verification wins on collision.
    // Only mark `verified_at_tick` when at least one row joined this scene —
    // a staleness anchor with nothing anchored would be noise.
    let mut verified_at_tick = None;
    if let Some(v) = verification {
        for (element_id, verdict, value) in &v.verdicts {
            let key = element_id.to_string();
            if find_node(&scene.nodes, &key).is_none() {
                continue;
            }
            elements.insert(key, ElementVerdict { verdict: Some(*verdict), value: *value });
            verified_at_tick = Some(v.verified_at_tick);
        }
    }

    // Defensive — every inserted entry has a verdict, so this is a no-op today.
    elements.retain(|_, e| !e.is_empty());

    VerdictOverlay {
        tick: snapshot.tick,
        time_ms: snapshot.time_ms,
        verified_at_tick,
        elements,
    }
}

/// Find the node with `element_id == id` anywhere in `nodes` (depth-first).
fn find_node<'a>(nodes: &'a [DiagramNode], id: &str) -> Option<&'a DiagramNode> {
    for node in nodes {
        if node.element_id == id {
            return Some(node);
        }
        if let Some(found) = node.children.iter().find_map(|c| find_child_node(c, id)) {
            return Some(found);
        }
    }
    None
}

fn find_child_node<'a>(child: &'a DiagramChild, id: &str) -> Option<&'a DiagramNode> {
    match child {
        DiagramChild::Node(n) => {
            if n.element_id == id {
                Some(n)
            } else {
                n.children.iter().find_map(|c| find_child_node(c, id))
            }
        }
        DiagramChild::Compartment { children, .. } => {
            children.iter().find_map(|c| find_child_node(c, id))
        }
        DiagramChild::Island { subtree, .. } => find_node(&subtree.nodes, id),
        DiagramChild::Text { .. } | DiagramChild::Edge(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use sysml_core::ElementId;
    use sysml_runtime::orchestrator::ConstraintEvalResult;

    use super::*;
    use crate::ViewType;
    use crate::visual_kind::VisualKind;

    /// Canonical scene-id string for a logical id — the same round-trip every
    /// real scene node makes (`element.id.to_string()`), so it equals
    /// `element_id.to_string()` in the join (UUID feature on by default).
    fn eid(id: &str) -> String {
        ElementId::from_string(id).to_string()
    }

    fn node(id: &str, name: &str) -> DiagramNode {
        DiagramNode::new(eid(id), VisualKind::Constraint, name)
    }

    fn constraint_result(
        name: &str,
        satisfied: bool,
        element_id: Option<&str>,
        operands: HashMap<String, f64>,
    ) -> ConstraintEvalResult {
        ConstraintEvalResult {
            name: name.to_owned(),
            verdict: if satisfied {
                VerdictKind::Pass
            } else {
                VerdictKind::Fail
            },
            expression: None,
            operands,
            element_id: element_id.map(ElementId::from_string),
        }
    }

    fn snapshot(tick: u64, constraint_results: Vec<ConstraintEvalResult>) -> ExecutionSnapshot {
        ExecutionSnapshot {
            tick,
            time_ms: tick as f64,
            subsystem_states: HashMap::new(),
            variables: Arc::new(HashMap::new()),
            messages: Vec::new(),
            constraint_results,
            assertion_checkpoints: Vec::new(),
            guard_diagnoses: Vec::new(),
            causation_links: Vec::new(),
            completed: false,
            port_values: HashMap::new(),
            derivatives: HashMap::new(),
            resolved_refs: HashMap::new(),
            flow_drop_warnings: Vec::new(),
            value_units: Arc::new(HashMap::new()),
            step_size_health: Vec::new(),
        }
    }

    /// A satisfied and an unsatisfied constraint both decorate by their real
    /// ElementIds; the single-operand one carries its solved value.
    #[test]
    fn pass_and_fail_keyed_by_element_id() {
        let scene = DiagramIR {
            view_type: ViewType::Interconnection,
            nodes: vec![node("c-ok", "MassBudget"), node("c-bad", "PowerBudget")],
            edges: Vec::new(),
            buttons: Vec::new(),
        };
        let snap = snapshot(
            5,
            vec![
                constraint_result(
                    "MassBudget",
                    true,
                    Some("c-ok"),
                    HashMap::from([("mass".to_owned(), 12.5)]),
                ),
                constraint_result("PowerBudget", false, Some("c-bad"), HashMap::new()),
            ],
        );

        let overlay = build_verdict_overlay(&scene, &snap, None);
        assert_eq!(overlay.tick, 5);
        assert_eq!(
            overlay.elements[&eid("c-ok")],
            ElementVerdict { verdict: Some(VerdictKind::Pass), value: Some(12.5) }
        );
        assert_eq!(
            overlay.elements[&eid("c-bad")],
            ElementVerdict { verdict: Some(VerdictKind::Fail), value: None }
        );
    }

    /// A constraint with multiple operands has no single solved value to badge.
    #[test]
    fn multiple_operands_yield_no_value() {
        let scene = DiagramIR {
            view_type: ViewType::Interconnection,
            nodes: vec![node("c-1", "C")],
            edges: Vec::new(),
            buttons: Vec::new(),
        };
        let snap = snapshot(
            1,
            vec![constraint_result(
                "C",
                true,
                Some("c-1"),
                HashMap::from([("a".to_owned(), 1.0), ("b".to_owned(), 2.0)]),
            )],
        );
        let overlay = build_verdict_overlay(&scene, &snap, None);
        assert_eq!(overlay.elements[&eid("c-1")].verdict, Some(VerdictKind::Pass));
        assert_eq!(overlay.elements[&eid("c-1")].value, None);
    }

    /// A result with no element_id has no honest join target — skipped.
    #[test]
    fn result_without_element_id_is_skipped() {
        let scene = DiagramIR {
            view_type: ViewType::Interconnection,
            nodes: vec![node("c-1", "C")],
            edges: Vec::new(),
            buttons: Vec::new(),
        };
        let snap = snapshot(1, vec![constraint_result("C", true, None, HashMap::new())]);
        let overlay = build_verdict_overlay(&scene, &snap, None);
        assert!(overlay.elements.is_empty());
    }

    /// Stored verification verdicts join scene nodes by ElementId, carry the
    /// full 4-variant VerdictKind, and surface `verified_at_tick`; off-scene
    /// rows are skipped (sparse, scene-scoped).
    #[test]
    fn verification_verdicts_join_and_carry_staleness_anchor() {
        let scene = DiagramIR {
            view_type: ViewType::General,
            nodes: vec![node("req-1", "TripAt1xRated"), node("req-2", "NuisanceFloor")],
            edges: Vec::new(),
            buttons: Vec::new(),
        };
        let snap = snapshot(30, Vec::new()); // session advanced past the verify
        let verification = VerificationVerdicts {
            verified_at_tick: 20,
            verdicts: vec![
                (ElementId::from_string(&eid("req-1")), VerdictKind::Fail, Some(0.43)),
                (ElementId::from_string(&eid("req-2")), VerdictKind::Inconclusive, None),
                (ElementId::from_string(&eid("req-ghost")), VerdictKind::Pass, None), // off-scene
            ],
        };
        let overlay = build_verdict_overlay(&scene, &snap, Some(&verification));
        assert_eq!(overlay.tick, 30);
        assert_eq!(overlay.verified_at_tick, Some(20)); // stale-but-labeled
        assert_eq!(
            overlay.elements[&eid("req-1")],
            ElementVerdict { verdict: Some(VerdictKind::Fail), value: Some(0.43) }
        );
        assert_eq!(
            overlay.elements[&eid("req-2")].verdict,
            Some(VerdictKind::Inconclusive)
        );
        assert_eq!(overlay.elements.len(), 2); // ghost skipped
    }

    /// PRECEDENCE GATE (steward Q3): a requirement's checks ARE constraint
    /// usages (Requirements.sysml `:> constraintChecks`), so the same element
    /// can appear in both the per-tick solver results and a stored
    /// verification. The verification verdict wins — it is the richer,
    /// user-initiated judgment.
    #[test]
    fn verification_wins_over_per_tick_constraint_result_on_collision() {
        let scene = DiagramIR {
            view_type: ViewType::Interconnection,
            nodes: vec![node("c-shared", "SharedCheck")],
            edges: Vec::new(),
            buttons: Vec::new(),
        };
        let snap = snapshot(
            10,
            vec![constraint_result(
                "SharedCheck",
                true, // solver says Pass this tick...
                Some("c-shared"),
                HashMap::from([("x".to_owned(), 1.0)]),
            )],
        );
        let verification = VerificationVerdicts {
            verified_at_tick: 10,
            verdicts: vec![(
                ElementId::from_string(&eid("c-shared")),
                VerdictKind::Fail, // ...but the verification judged Fail.
                None,
            )],
        };
        let overlay = build_verdict_overlay(&scene, &snap, Some(&verification));
        assert_eq!(overlay.elements[&eid("c-shared")].verdict, Some(VerdictKind::Fail));
    }

    /// No verification rows joining this scene → no staleness anchor (an
    /// anchor with nothing anchored would be noise).
    #[test]
    fn verified_at_tick_absent_when_no_verification_row_joins() {
        let scene = DiagramIR {
            view_type: ViewType::Interconnection,
            nodes: vec![node("c-1", "C")],
            edges: Vec::new(),
            buttons: Vec::new(),
        };
        let snap = snapshot(1, Vec::new());
        let verification = VerificationVerdicts {
            verified_at_tick: 1,
            verdicts: vec![(ElementId::from_string(&eid("off-scene")), VerdictKind::Pass, None)],
        };
        let overlay = build_verdict_overlay(&scene, &snap, Some(&verification));
        assert!(overlay.elements.is_empty());
        assert_eq!(overlay.verified_at_tick, None);
    }

    /// A result whose id is not in the scene is skipped (sparse, scene-scoped).
    #[test]
    fn result_off_scene_is_skipped() {
        let scene = DiagramIR {
            view_type: ViewType::Interconnection,
            nodes: vec![node("c-1", "C")],
            edges: Vec::new(),
            buttons: Vec::new(),
        };
        let snap = snapshot(
            1,
            vec![constraint_result("Ghost", false, Some("c-ghost"), HashMap::new())],
        );
        let overlay = build_verdict_overlay(&scene, &snap, None);
        assert!(overlay.elements.is_empty());
    }
}
