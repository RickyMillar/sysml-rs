//! Delta frames over [`NormalizedSnapshot`].
//!
//! A streaming client receives one full snapshot at connect and a
//! [`DeltaFrame`] per tick thereafter. Applying the delta to the prior
//! snapshot reconstructs the next snapshot on the client side without a
//! fresh round-trip of the full variable map.
//!
//! The diff is intentionally tick-coarse: we replace the constraint list
//! wholesale whenever it changes (these rows are O(constraints), not
//! O(variables), so the simplicity is free) but do per-key diffing on
//! scalar / string vars and subsystems, which are the large maps.
//!

use std::collections::HashMap;

use crate::cases::VerdictKind;
use crate::snapshot_view::{ConstraintView, NormalizedSnapshot, SubsystemView};

/// Diff between two consecutive [`NormalizedSnapshot`]s.
///
/// `apply(prev, &diff(prev, next))` reconstructs `next` exactly. If
/// `prev` is `None`, the delta carries the full frame (every scalar /
/// string / subsystem from `next` appears under `*_changed`).
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DeltaFrame {
    pub tick: u64,
    pub time_ms: f64,
    pub completed: bool,

    /// Scalar vars added or whose value changed since `prev`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "HashMap::is_empty")
    )]
    pub scalar_changed: HashMap<String, f64>,
    /// Scalar var keys removed since `prev`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub scalar_removed: Vec<String>,

    /// String vars added or whose value changed since `prev`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "HashMap::is_empty")
    )]
    pub string_changed: HashMap<String, String>,
    /// String var keys removed since `prev`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub string_removed: Vec<String>,

    /// Subsystems added or whose view changed since `prev`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "HashMap::is_empty")
    )]
    pub subsystem_changed: HashMap<String, SubsystemView>,
    /// Subsystem keys removed since `prev`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub subsystem_removed: Vec<String>,

    /// Constraint result rows (wholesale replacement when any row changed).
    /// `None` means "unchanged, reuse prev's rows".
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub constraint_results: Option<Vec<ConstraintView>>,

    /// Ports whose feature map (keyed as `owner.port` → `feature` →
    /// `f64`) added a key or changed any feature value since `prev`.
    /// The value is the full feature map for the port (wholesale
    /// replacement) — port feature sets are small (~2-4 entries) so
    /// per-feature diffing isn't worth the complexity. Closes
    /// GAP-FLOW-001 on the delta side.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "HashMap::is_empty")
    )]
    pub port_values_changed: HashMap<String, HashMap<String, f64>>,
    /// Port keys removed since `prev`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub port_values_removed: Vec<String>,

    /// ODE state-variable derivatives (`dy/dt`) added or whose value
    /// changed since `prev`. Same add/change/remove semantics as
    /// `scalar_vars` — closes GAP-ODE-002 on the delta side.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "HashMap::is_empty")
    )]
    pub derivatives_changed: HashMap<String, f64>,
    /// Derivative keys removed since `prev` (state var left the ODE set).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub derivatives_removed: Vec<String>,
}

/// Compute a [`DeltaFrame`] from `prev` → `next`.
///
/// Passing `prev = None` is the "hello frame" case: every key in `next`
/// appears under `*_changed` and nothing is marked removed.
pub fn diff(prev: Option<&NormalizedSnapshot>, next: &NormalizedSnapshot) -> DeltaFrame {
    let empty = NormalizedSnapshot::default();
    let prev = prev.unwrap_or(&empty);

    let (scalar_changed, scalar_removed) = diff_map(&prev.scalar_vars, &next.scalar_vars);
    let (string_changed, string_removed) = diff_map(&prev.string_vars, &next.string_vars);
    let (subsystem_changed, subsystem_removed) = diff_map(&prev.subsystems, &next.subsystems);
    let (port_values_changed, port_values_removed) = diff_map(&prev.port_values, &next.port_values);
    let (derivatives_changed, derivatives_removed) = diff_map(&prev.derivatives, &next.derivatives);

    let constraint_results = if prev.constraint_results == next.constraint_results {
        None
    } else {
        Some(next.constraint_results.clone())
    };

    DeltaFrame {
        tick: next.tick,
        time_ms: next.time_ms,
        completed: next.completed,
        scalar_changed,
        scalar_removed,
        string_changed,
        string_removed,
        subsystem_changed,
        subsystem_removed,
        constraint_results,
        port_values_changed,
        port_values_removed,
        derivatives_changed,
        derivatives_removed,
    }
}

/// Apply `delta` to `base` in place, producing the snapshot that `delta`
/// was computed against.
pub fn apply(base: &mut NormalizedSnapshot, delta: &DeltaFrame) {
    base.tick = delta.tick;
    base.time_ms = delta.time_ms;
    base.completed = delta.completed;

    for (k, v) in &delta.scalar_changed {
        base.scalar_vars.insert(k.clone(), *v);
    }
    for k in &delta.scalar_removed {
        base.scalar_vars.remove(k);
    }

    for (k, v) in &delta.string_changed {
        base.string_vars.insert(k.clone(), v.clone());
    }
    for k in &delta.string_removed {
        base.string_vars.remove(k);
    }

    for (k, v) in &delta.subsystem_changed {
        base.subsystems.insert(k.clone(), v.clone());
    }
    for k in &delta.subsystem_removed {
        base.subsystems.remove(k);
    }

    if let Some(rows) = &delta.constraint_results {
        base.constraint_results = rows.clone();
    }

    for (k, v) in &delta.port_values_changed {
        base.port_values.insert(k.clone(), v.clone());
    }
    for k in &delta.port_values_removed {
        base.port_values.remove(k);
    }

    for (k, v) in &delta.derivatives_changed {
        base.derivatives.insert(k.clone(), *v);
    }
    for k in &delta.derivatives_removed {
        base.derivatives.remove(k);
    }
}

/// Given `prev` and `next`, return `(added_or_changed, removed_keys)`.
fn diff_map<V>(
    prev: &HashMap<String, V>,
    next: &HashMap<String, V>,
) -> (HashMap<String, V>, Vec<String>)
where
    V: Clone + PartialEq,
{
    let mut changed = HashMap::new();
    for (k, v) in next {
        match prev.get(k) {
            Some(pv) if pv == v => {}
            _ => {
                changed.insert(k.clone(), v.clone());
            }
        }
    }
    let removed: Vec<String> = prev
        .keys()
        .filter(|k| !next.contains_key(*k))
        .cloned()
        .collect();
    (changed, removed)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::snapshot_view::{ConstraintView, NormalizedSnapshot, SubsystemView};

    fn snap(
        tick: u64,
        time_ms: f64,
        scalars: &[(&str, f64)],
        strings: &[(&str, &str)],
        subsystems: &[(&str, &str, &str, bool)],
    ) -> NormalizedSnapshot {
        let mut s = NormalizedSnapshot {
            tick,
            time_ms,
            completed: false,
            ..Default::default()
        };
        for (k, v) in scalars {
            s.scalar_vars.insert((*k).into(), *v);
        }
        for (k, v) in strings {
            s.string_vars.insert((*k).into(), (*v).into());
        }
        for (name, state, kind, done) in subsystems {
            s.subsystems.insert(
                (*name).into(),
                SubsystemView {
                    current_state: (*state).into(),
                    kind_label: (*kind).into(),
                    completed: *done,
                    available_transitions: Vec::new(),
                    element_id: None,
                },
            );
        }
        s
    }

    #[test]
    fn diff_hello_frame_carries_everything() {
        let next = snap(
            1,
            100.0,
            &[("a", 1.0), ("b", 2.0)],
            &[("mode", "normal")],
            &[("sm", "Idle", "stateMachine", false)],
        );

        let delta = diff(None, &next);

        assert_eq!(delta.tick, 1);
        assert_eq!(delta.time_ms, 100.0);
        assert_eq!(delta.scalar_changed.len(), 2);
        assert_eq!(delta.scalar_changed.get("a"), Some(&1.0));
        assert_eq!(delta.scalar_changed.get("b"), Some(&2.0));
        assert!(delta.scalar_removed.is_empty());
        assert_eq!(delta.string_changed.len(), 1);
        assert_eq!(delta.subsystem_changed.len(), 1);
        // No prior rows, so we emit whatever next has (empty in this case)
        // and leave constraint_results = None iff both are empty, which they are.
        assert!(delta.constraint_results.is_none());
    }

    #[test]
    fn diff_records_added_changed_and_removed() {
        let a = snap(
            1,
            100.0,
            &[("x", 1.0), ("y", 2.0), ("z", 3.0)],
            &[("mode", "normal"), ("label", "A")],
            &[("sm1", "Idle", "stateMachine", false)],
        );
        let b = snap(
            2,
            200.0,
            // x unchanged, y changed, z removed, w added
            &[("x", 1.0), ("y", 2.5), ("w", 9.0)],
            // mode unchanged, label removed, extra added
            &[("mode", "normal"), ("extra", "B")],
            // sm1 changed state, sm2 added
            &[
                ("sm1", "Running", "stateMachine", false),
                ("sm2", "Idle", "action", false),
            ],
        );

        let delta = diff(Some(&a), &b);
        assert_eq!(delta.tick, 2);
        assert_eq!(delta.time_ms, 200.0);

        assert!(!delta.scalar_changed.contains_key("x"));
        assert_eq!(delta.scalar_changed.get("y"), Some(&2.5));
        assert_eq!(delta.scalar_changed.get("w"), Some(&9.0));
        assert!(delta.scalar_removed.iter().any(|k| k == "z"));

        assert!(!delta.string_changed.contains_key("mode"));
        assert_eq!(
            delta.string_changed.get("extra").map(|s| s.as_str()),
            Some("B"),
        );
        assert!(delta.string_removed.iter().any(|k| k == "label"));

        assert_eq!(delta.subsystem_changed.len(), 2);
        assert_eq!(
            delta
                .subsystem_changed
                .get("sm1")
                .map(|s| s.current_state.as_str()),
            Some("Running"),
        );
        assert!(delta.subsystem_changed.contains_key("sm2"));
        assert!(delta.subsystem_removed.is_empty());
    }

    #[test]
    fn apply_round_trip_reconstructs_next() {
        let a = snap(
            1,
            100.0,
            &[("x", 1.0), ("y", 2.0), ("z", 3.0)],
            &[("mode", "normal"), ("label", "A")],
            &[("sm1", "Idle", "stateMachine", false)],
        );
        let b = snap(
            2,
            200.0,
            &[("x", 1.0), ("y", 2.5), ("w", 9.0)],
            &[("mode", "normal"), ("extra", "B")],
            &[
                ("sm1", "Running", "stateMachine", false),
                ("sm2", "Idle", "action", false),
            ],
        );

        let delta = diff(Some(&a), &b);
        let mut base = a;
        apply(&mut base, &delta);
        assert_eq!(base, b);
    }

    #[test]
    fn apply_on_empty_base_uses_hello_frame() {
        let next = snap(
            5,
            500.0,
            &[("a", 1.0)],
            &[],
            &[("sm", "Idle", "stateMachine", false)],
        );
        let delta = diff(None, &next);
        let mut base = NormalizedSnapshot::default();
        apply(&mut base, &delta);
        assert_eq!(base, next);
    }

    #[test]
    fn constraint_results_replaced_when_changed() {
        let mut a = snap(1, 100.0, &[], &[], &[]);
        a.constraint_results = vec![ConstraintView {
            name: "c1".into(),
            expression: Some("x < 10".into()),
            verdict: VerdictKind::Pass,
            operands: HashMap::new(),
            element_id: None,
        }];
        let mut b = snap(2, 200.0, &[], &[], &[]);
        b.constraint_results = vec![ConstraintView {
            name: "c1".into(),
            expression: Some("x < 10".into()),
            verdict: VerdictKind::Fail,
            operands: HashMap::new(),
            element_id: None,
        }];

        let delta = diff(Some(&a), &b);
        assert!(delta.constraint_results.is_some());
        let replacement = delta.constraint_results.as_ref().expect("some");
        assert_eq!(replacement.len(), 1);
        assert_eq!(replacement[0].verdict, VerdictKind::Fail);

        let mut base = a;
        apply(&mut base, &delta);
        assert_eq!(base.constraint_results, b.constraint_results);
    }

    #[test]
    fn port_values_diff_and_apply_round_trip() {
        // GAP-FLOW-001: port maps participate in the delta frame with
        // the same add/change/remove semantics as scalar_vars.
        let mut a = snap(1, 100.0, &[], &[], &[]);
        a.port_values.insert(
            "tank.waterOut".into(),
            [("flowRate".into(), 1.0), ("pressure".into(), 100.0)]
                .into_iter()
                .collect(),
        );
        a.port_values.insert(
            "pump.in".into(),
            [("flowRate".into(), 2.0)].into_iter().collect(),
        );
        let mut b = snap(2, 200.0, &[], &[], &[]);
        // Changed: flowRate increased on tank.waterOut.
        b.port_values.insert(
            "tank.waterOut".into(),
            [("flowRate".into(), 1.5), ("pressure".into(), 100.0)]
                .into_iter()
                .collect(),
        );
        // Added: new port.
        b.port_values.insert(
            "valve.inlet".into(),
            [("flowRate".into(), 0.8)].into_iter().collect(),
        );
        // Removed: pump.in gone.

        let delta = diff(Some(&a), &b);
        assert!(delta.port_values_changed.contains_key("tank.waterOut"));
        assert!(delta.port_values_changed.contains_key("valve.inlet"));
        assert!(!delta.port_values_changed.contains_key("pump.in"));
        assert_eq!(delta.port_values_removed, vec!["pump.in".to_string()]);

        let mut base = a;
        apply(&mut base, &delta);
        assert_eq!(base.port_values, b.port_values);
    }

    #[test]
    fn derivatives_diff_and_apply_round_trip() {
        // GAP-ODE-002: dy/dt participates in the delta frame with the
        // same add/change/remove semantics as scalar_vars.
        let mut a = snap(1, 100.0, &[], &[], &[]);
        a.derivatives.insert("T_bus".into(), 0.1);
        a.derivatives.insert("charge".into(), -0.02);

        let mut b = snap(2, 200.0, &[], &[], &[]);
        // T_bus changed, charge removed, new_state added.
        b.derivatives.insert("T_bus".into(), 0.15);
        b.derivatives.insert("new_state".into(), 0.5);

        let delta = diff(Some(&a), &b);
        assert_eq!(delta.derivatives_changed.get("T_bus"), Some(&0.15));
        assert_eq!(delta.derivatives_changed.get("new_state"), Some(&0.5));
        assert!(!delta.derivatives_changed.contains_key("charge"));
        assert_eq!(delta.derivatives_removed, vec!["charge".to_string()]);

        let mut base = a;
        apply(&mut base, &delta);
        assert_eq!(base.derivatives, b.derivatives);
    }

    #[test]
    fn port_values_unchanged_omits_from_delta() {
        let mut a = snap(1, 100.0, &[], &[], &[]);
        a.port_values.insert(
            "tank.waterOut".into(),
            [("flowRate".into(), 1.0)].into_iter().collect(),
        );
        let mut b = snap(2, 200.0, &[], &[], &[]);
        b.port_values = a.port_values.clone();

        let delta = diff(Some(&a), &b);
        assert!(delta.port_values_changed.is_empty());
        assert!(delta.port_values_removed.is_empty());
    }

    #[test]
    fn constraint_results_unchanged_emits_none() {
        let row = ConstraintView {
            name: "c1".into(),
            expression: None,
            verdict: VerdictKind::Pass,
            operands: HashMap::new(),
            element_id: None,
        };
        let mut a = snap(1, 100.0, &[], &[], &[]);
        a.constraint_results = vec![row.clone()];
        let mut b = snap(2, 200.0, &[], &[], &[]);
        b.constraint_results = vec![row];

        let delta = diff(Some(&a), &b);
        assert!(delta.constraint_results.is_none());

        let mut base = a.clone();
        apply(&mut base, &delta);
        assert_eq!(base.constraint_results, a.constraint_results);
    }

    #[test]
    fn completed_flag_propagates_through_delta() {
        let a = snap(1, 100.0, &[], &[], &[]);
        let mut b = snap(2, 200.0, &[], &[], &[]);
        b.completed = true;

        let delta = diff(Some(&a), &b);
        assert!(delta.completed);
        let mut base = a;
        apply(&mut base, &delta);
        assert!(base.completed);
    }
}
