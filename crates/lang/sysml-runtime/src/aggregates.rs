//! Aggregation over a sequence of normalized snapshots.
//!
//! Consumers (results panels, analysis reports, CLI `sim-tail --summary`)
//! need rolled-up metrics rather than per-tick state: min / max / mean for
//! each scalar variable, a verdict rollup from the constraint rows, and
//! the time range the run actually covered. This module is the canonical
//! Rust-side computation — the frontend's `selectKPISummaries` can drop
//! its bespoke heuristics in Stage 6 once it consumes these directly.
//!
//! The unit/label heuristics stay client-side (a render concern); this
//! module only produces raw statistics per variable.
//!

use std::collections::HashMap;

use crate::cases::VerdictKind;
use crate::snapshot_view::{ConstraintView, NormalizedSnapshot};

/// Per-variable statistics across a run.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct KpiSummary {
    pub variable: String,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    /// Max `|v|` — useful for AC waveforms where peak amplitude matters.
    pub peak_abs: f64,
    pub first: f64,
    pub last: f64,
    pub sample_count: usize,
}

/// Count of constraint / verdict outcomes across the four `VerdictKind`
/// values. Every producer records a real verdict — there is no bool
/// shortcut, because collapsing an undecided constraint into `fail` is
/// what made session badges report library noise as violations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct VerdictRollup {
    pub pass: usize,
    pub fail: usize,
    pub inconclusive: usize,
    pub error: usize,
}

impl VerdictRollup {
    pub fn total(&self) -> usize {
        self.pass + self.fail + self.inconclusive + self.error
    }

    /// Worst overall verdict across the rollup. Empty → `Pass`.
    pub fn overall(&self) -> VerdictKind {
        if self.error > 0 {
            VerdictKind::Error
        } else if self.fail > 0 {
            VerdictKind::Fail
        } else if self.inconclusive > 0 {
            VerdictKind::Inconclusive
        } else {
            VerdictKind::Pass
        }
    }

    pub fn record(&mut self, verdict: VerdictKind) {
        match verdict {
            VerdictKind::Pass => self.pass += 1,
            VerdictKind::Fail => self.fail += 1,
            VerdictKind::Inconclusive => self.inconclusive += 1,
            VerdictKind::Error => self.error += 1,
        }
    }
}

/// The full aggregate metrics payload.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AggregateMetrics {
    pub tick_count: usize,
    /// `(first_time_ms, last_time_ms)`. `(0.0, 0.0)` when no frames.
    pub time_range: (f64, f64),
    /// Per-variable stats, keyed by variable name.
    pub kpis: HashMap<String, KpiSummary>,
    /// Verdict rollup over the final frame's constraint rows.
    pub verdicts: VerdictRollup,
    /// Final observed `current_state` for each subsystem (empty when no frames).
    pub final_subsystem_states: HashMap<String, String>,
}

/// Compute aggregate metrics over a run's frames.
///
/// Frames are assumed to be in tick order. The verdict rollup uses the
/// *final* frame's constraint rows — per-tick constraint churn doesn't
/// compound well (a failing constraint at t=100ms that recovers by
/// t=200ms isn't a failure). Callers who want per-tick aggregation can
/// drive [`VerdictRollup::record`] themselves.
pub fn aggregate(frames: &[NormalizedSnapshot]) -> AggregateMetrics {
    if frames.is_empty() {
        return AggregateMetrics::default();
    }

    let first = &frames[0];
    let last = &frames[frames.len() - 1];

    // Per-variable streaming stats (avoids second pass over frames).
    let mut kpis: HashMap<String, KpiSummary> = HashMap::new();
    let mut sums: HashMap<String, f64> = HashMap::new();

    for frame in frames {
        for (name, value) in &frame.scalar_vars {
            let entry = kpis.entry(name.clone()).or_insert_with(|| KpiSummary {
                variable: name.clone(),
                min: *value,
                max: *value,
                mean: 0.0,
                peak_abs: value.abs(),
                first: *value,
                last: *value,
                sample_count: 0,
            });
            if !value.is_nan() {
                if *value < entry.min || entry.sample_count == 0 {
                    entry.min = *value;
                }
                if *value > entry.max || entry.sample_count == 0 {
                    entry.max = *value;
                }
                if value.abs() > entry.peak_abs || entry.sample_count == 0 {
                    entry.peak_abs = value.abs();
                }
                entry.last = *value;
                entry.sample_count += 1;
                *sums.entry(name.clone()).or_insert(0.0) += *value;
            }
        }
    }

    for (name, entry) in kpis.iter_mut() {
        if entry.sample_count > 0 {
            let sum = sums.get(name).copied().unwrap_or(0.0);
            entry.mean = sum / entry.sample_count as f64;
        }
    }

    let verdicts = verdict_rollup_from_constraints(&last.constraint_results);

    let final_subsystem_states = last
        .subsystems
        .iter()
        .map(|(k, v)| (k.clone(), v.current_state.clone()))
        .collect();

    AggregateMetrics {
        tick_count: frames.len(),
        time_range: (first.time_ms, last.time_ms),
        kpis,
        verdicts,
        final_subsystem_states,
    }
}

/// Build a [`VerdictRollup`] from a constraint-result slice.
///
/// Constraint rows carry a full [`VerdictKind`], so undecidable rows land in
/// `inconclusive` instead of inflating `fail` — the rollup, and every badge
/// downstream of it, counts only constraints the run actually decided
/// against.
pub fn verdict_rollup_from_constraints(rows: &[ConstraintView]) -> VerdictRollup {
    let mut rollup = VerdictRollup::default();
    for row in rows {
        rollup.record(row.verdict);
    }
    rollup
}

/// Build a [`VerdictRollup`] from a real `VerdictKind` slice. Used by
/// verification-case consumers.
pub fn verdict_rollup_from_verdicts(verdicts: &[VerdictKind]) -> VerdictRollup {
    let mut rollup = VerdictRollup::default();
    for v in verdicts {
        rollup.record(*v);
    }
    rollup
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::snapshot_view::{ConstraintView, NormalizedSnapshot, SubsystemView};

    fn frame(tick: u64, time_ms: f64, scalars: &[(&str, f64)]) -> NormalizedSnapshot {
        let mut s = NormalizedSnapshot {
            tick,
            time_ms,
            ..Default::default()
        };
        for (k, v) in scalars {
            s.scalar_vars.insert((*k).into(), *v);
        }
        s
    }

    #[test]
    fn aggregate_empty_returns_default() {
        let m = aggregate(&[]);
        assert_eq!(m.tick_count, 0);
        assert_eq!(m.time_range, (0.0, 0.0));
        assert!(m.kpis.is_empty());
        assert_eq!(m.verdicts.total(), 0);
    }

    #[test]
    fn aggregate_computes_min_max_mean_peak() {
        let frames = [
            frame(0, 0.0, &[("x", 1.0)]),
            frame(1, 10.0, &[("x", -5.0)]),
            frame(2, 20.0, &[("x", 3.0)]),
        ];
        let m = aggregate(&frames);
        assert_eq!(m.tick_count, 3);
        assert_eq!(m.time_range, (0.0, 20.0));
        let k = m.kpis.get("x").expect("x present");
        assert_eq!(k.min, -5.0);
        assert_eq!(k.max, 3.0);
        assert!((k.mean - (1.0 - 5.0 + 3.0) / 3.0).abs() < 1e-12);
        assert_eq!(k.peak_abs, 5.0);
        assert_eq!(k.first, 1.0);
        assert_eq!(k.last, 3.0);
        assert_eq!(k.sample_count, 3);
    }

    #[test]
    fn aggregate_skips_nan_samples() {
        let frames = [
            frame(0, 0.0, &[("x", f64::NAN), ("y", 2.0)]),
            frame(1, 10.0, &[("x", 1.0), ("y", 4.0)]),
            frame(2, 20.0, &[("x", 3.0), ("y", 6.0)]),
        ];
        let m = aggregate(&frames);
        let x = m.kpis.get("x").expect("x");
        assert_eq!(x.sample_count, 2);
        assert_eq!(x.min, 1.0);
        assert_eq!(x.max, 3.0);
        assert!((x.mean - 2.0).abs() < 1e-12);

        let y = m.kpis.get("y").expect("y");
        assert_eq!(y.sample_count, 3);
        assert!((y.mean - 4.0).abs() < 1e-12);
    }

    #[test]
    fn aggregate_handles_series_that_appears_midrun() {
        let frames = [
            frame(0, 0.0, &[("a", 1.0)]),
            frame(1, 10.0, &[("a", 2.0), ("b", 42.0)]),
            frame(2, 20.0, &[("a", 3.0), ("b", 43.0)]),
        ];
        let m = aggregate(&frames);
        let b = m.kpis.get("b").expect("b");
        assert_eq!(b.sample_count, 2);
        assert_eq!(b.first, 42.0);
        assert_eq!(b.last, 43.0);
    }

    #[test]
    fn aggregate_rolls_up_final_frame_constraints_and_subsystems() {
        let mut frames = [frame(0, 0.0, &[]), frame(1, 10.0, &[]), frame(2, 20.0, &[])];
        frames[0].constraint_results = vec![ConstraintView {
            name: "c1".into(),
            verdict: VerdictKind::Pass,
            expression: None,
            operands: Default::default(),
            element_id: None,
        }];
        frames[2].constraint_results = vec![
            ConstraintView {
                name: "c1".into(),
                verdict: VerdictKind::Pass,
                expression: None,
                operands: Default::default(),
                element_id: None,
            },
            ConstraintView {
                name: "c2".into(),
                verdict: VerdictKind::Fail,
                expression: None,
                operands: Default::default(),
                element_id: None,
            },
        ];
        frames[2].subsystems.insert(
            "sm1".into(),
            SubsystemView {
                current_state: "Tripped".into(),
                kind_label: "stateMachine".into(),
                completed: false,
                available_transitions: Vec::new(),
                element_id: None,
            },
        );

        let m = aggregate(&frames);
        assert_eq!(m.verdicts.pass, 1);
        assert_eq!(m.verdicts.fail, 1);
        assert_eq!(m.verdicts.total(), 2);
        assert_eq!(m.verdicts.overall(), VerdictKind::Fail);
        assert_eq!(
            m.final_subsystem_states.get("sm1").map(String::as_str),
            Some("Tripped"),
        );
    }

    #[test]
    fn verdict_rollup_overall_priority() {
        let mut r = VerdictRollup::default();
        assert_eq!(r.overall(), VerdictKind::Pass);
        r.record(VerdictKind::Inconclusive);
        assert_eq!(r.overall(), VerdictKind::Inconclusive);
        r.record(VerdictKind::Fail);
        assert_eq!(r.overall(), VerdictKind::Fail);
        r.record(VerdictKind::Error);
        assert_eq!(r.overall(), VerdictKind::Error);
    }

    #[test]
    fn verdict_rollup_from_verdicts_helper() {
        let r = verdict_rollup_from_verdicts(&[
            VerdictKind::Pass,
            VerdictKind::Pass,
            VerdictKind::Fail,
            VerdictKind::Inconclusive,
        ]);
        assert_eq!(r.pass, 2);
        assert_eq!(r.fail, 1);
        assert_eq!(r.inconclusive, 1);
        assert_eq!(r.error, 0);
        assert_eq!(r.overall(), VerdictKind::Fail);
    }
}
