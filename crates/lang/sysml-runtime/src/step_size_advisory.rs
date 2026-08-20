//! Step-size (under-resolution) advisory — a runtime numerical honesty
//! guardrail (P1 dt-under-resolution arc).
//!
//! # What this is
//!
//! When a hybrid discrete/continuous model oscillates, the *period* the user
//! sees can be **step-bound, not physics-bound**: at coarse `dt` the state
//! machine only samples the continuous state a handful of times per cycle, so
//! the observed waveform is a numerical artifact of the tick rate rather than
//! the model's physics. The legacy oscillator fixture oscillates at ~5–8 ticks/cycle at
//! *every* `dt` — the plausible-looking square wave is under-resolved and
//! nothing warns the user.
//!
//! This type carries an **advisory** (not a diagnostic, not an error) computed
//! while stepping, mirroring the [`crate::statemachine::GuardDiagnosis`]
//! precedent: it is derived from tick bookkeeping, is purely observational, and
//! never influences stepping. It is emitted per ODE subsystem via a new
//! `step_size_health` field on [`crate::orchestrator::ExecutionSnapshot`],
//! surfaced through the session contract exactly the way `guard_diagnoses` is.
//!
//! # What this is NOT
//!
//! - Not a `Severity::Warning` / `Diagnostic`. The model is not in question.
//! - Not coupled into `Orchestrator::step`'s `dt` selection. It never changes
//!   stepping behaviour and never auto-changes `dt`. Fixing the *physics*
//!   (re-stepping the ODE at a finer internal `dt`) is a separate arc
//!   (RSC-4.3); this is only the honesty guardrail that tells the user.

/// Target number of ticks per oscillation cycle below which a model is flagged
/// as under-resolving its discrete/continuous coupling.
///
/// **This is a tooling heuristic, NOT derived from the model or the SysML v2
/// spec** — the spec is silent on step-size adequacy. A commonly cited rule of
/// thumb for resolving a periodic signal on a fixed grid is 20–50 samples per
/// cycle; we pick the low end (`20`) deliberately so the advisory fires only
/// when a waveform is clearly step-bound, minimising false positives.
///
/// It **can** false-positive on a model that genuinely cycles that fast — that
/// is an accepted tradeoff of a heuristic, not a certainty claim. The advisory
/// wording says so explicitly ("Model behavior is not in question").
pub const TARGET_TICKS_PER_CYCLE: u32 = 20;

/// Per-ODE-subsystem step-size health, computed while stepping.
///
/// "No crossings observed" is an **explicit distinct state**
/// ([`StepSizeAdvisory::NotApplicable`]) rather than a silent default to
/// "healthy": absence of an oscillation signal must never be read as a
/// resolution guarantee (fail-hard clarity).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
pub enum StepSizeAdvisory {
    /// No located zero-crossings have been observed for this subsystem yet, so
    /// there is no oscillation cycle to measure. This is NOT a claim that the
    /// step size is adequate — it is the explicit "no signal" state.
    NotApplicable,
    /// An oscillation cycle was observed and is adequately resolved
    /// (`ticks_per_cycle >= TARGET_TICKS_PER_CYCLE`).
    Ok {
        /// Observed oscillation period, in ticks.
        ticks_per_cycle: u32,
    },
    /// An oscillation cycle was observed but is step-bound: it resolves to
    /// fewer than [`TARGET_TICKS_PER_CYCLE`] ticks, so the waveform is likely
    /// a numerical artifact of the tick rate.
    UnderResolved {
        /// Observed oscillation period, in ticks.
        ticks_per_cycle: u32,
        /// A `dt` (in ms) that would resolve the observed period to
        /// approximately [`TARGET_TICKS_PER_CYCLE`] ticks/cycle:
        /// `observed_period_ms / TARGET_TICKS_PER_CYCLE`, where
        /// `observed_period_ms = ticks_per_cycle * dt_ms`.
        suggested_dt_ms: f64,
    },
}

impl StepSizeAdvisory {
    /// Classify an observed cycle length against [`TARGET_TICKS_PER_CYCLE`].
    ///
    /// `ticks_per_cycle == 0` (no measurable cycle) maps to
    /// [`StepSizeAdvisory::NotApplicable`]. Given a positive cycle length and
    /// the `dt_ms` it was observed at, returns `Ok` when the cycle is
    /// adequately resolved and `UnderResolved` (carrying the suggested `dt`)
    /// otherwise.
    pub fn classify(ticks_per_cycle: u32, dt_ms: f64) -> Self {
        if ticks_per_cycle == 0 {
            return StepSizeAdvisory::NotApplicable;
        }
        if ticks_per_cycle >= TARGET_TICKS_PER_CYCLE {
            StepSizeAdvisory::Ok { ticks_per_cycle }
        } else {
            let observed_period_ms = ticks_per_cycle as f64 * dt_ms;
            let suggested_dt_ms = observed_period_ms / TARGET_TICKS_PER_CYCLE as f64;
            StepSizeAdvisory::UnderResolved {
                ticks_per_cycle,
                suggested_dt_ms,
            }
        }
    }

    /// `true` only for [`StepSizeAdvisory::UnderResolved`]. Convenience for
    /// callers (tests, the service sink) that only care whether the advisory
    /// tripped.
    pub fn is_under_resolved(&self) -> bool {
        matches!(self, StepSizeAdvisory::UnderResolved { .. })
    }
}

/// One [`StepSizeAdvisory`] paired with the subsystem it describes, as carried
/// in [`crate::orchestrator::ExecutionSnapshot::step_size_health`].
///
/// Mirrors the "one entry per subsystem" shape of `guard_diagnoses`. The
/// `subsystem` name is the ODE subsystem's display name (resolved from its
/// `SubsystemIndex` at capture time).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SubsystemStepSizeHealth {
    /// Display name of the ODE subsystem this advisory describes.
    pub subsystem: String,
    /// The advisory for this subsystem.
    pub advisory: StepSizeAdvisory,
}

impl SubsystemStepSizeHealth {
    /// Render the advisory as user-facing text.
    ///
    /// Only [`StepSizeAdvisory::UnderResolved`] produces a message (the honest
    /// case worth surfacing); `Ok` and `NotApplicable` return `None`. The
    /// wording is deliberately advisory — never "error", never "the model has a
    /// bug" — and always carries the suggested `dt`.
    ///
    /// `session_id` is the session the advisory belongs to; `dt_ms` is the step
    /// size the cycle was observed at.
    pub fn message(&self, session_id: &str, dt_ms: f64) -> Option<String> {
        match &self.advisory {
            StepSizeAdvisory::UnderResolved {
                ticks_per_cycle,
                suggested_dt_ms,
            } => Some(format!(
                "Session {session_id}, subsystem {name}: observed cycle length \u{2248}{k} \
                 ticks at dt={dt}ms \u{2014} this under-resolves the discrete-continuous \
                 coupling (target \u{2265}{n} ticks/cycle). Consider dt \u{2248} {sugg:.4}ms. \
                 Model behavior is not in question; this is a numerical step-size advisory.",
                name = self.subsystem,
                k = ticks_per_cycle,
                dt = dt_ms,
                n = TARGET_TICKS_PER_CYCLE,
                sugg = suggested_dt_ms,
            )),
            StepSizeAdvisory::Ok { .. } | StepSizeAdvisory::NotApplicable => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_cycle_is_not_applicable() {
        assert_eq!(
            StepSizeAdvisory::classify(0, 1.0),
            StepSizeAdvisory::NotApplicable
        );
    }

    #[test]
    fn well_resolved_cycle_is_ok() {
        // 40 ticks/cycle >= target 20 => Ok.
        assert_eq!(
            StepSizeAdvisory::classify(40, 1.0),
            StepSizeAdvisory::Ok {
                ticks_per_cycle: 40
            }
        );
        // Exactly at the boundary is Ok, not under-resolved.
        assert_eq!(
            StepSizeAdvisory::classify(TARGET_TICKS_PER_CYCLE, 1.0),
            StepSizeAdvisory::Ok {
                ticks_per_cycle: TARGET_TICKS_PER_CYCLE
            }
        );
    }

    #[test]
    fn step_bound_cycle_is_under_resolved_with_suggestion() {
        // 6 ticks/cycle at dt=1ms => period 6ms; to hit 20 ticks/cycle,
        // dt = 6 / 20 = 0.3ms.
        let a = StepSizeAdvisory::classify(6, 1.0);
        match a {
            StepSizeAdvisory::UnderResolved {
                ticks_per_cycle,
                suggested_dt_ms,
            } => {
                assert_eq!(ticks_per_cycle, 6);
                assert!((suggested_dt_ms - 0.3).abs() < 1e-9);
            }
            other => panic!("expected UnderResolved, got {other:?}"),
        }
    }

    #[test]
    fn message_only_for_under_resolved() {
        let ok = SubsystemStepSizeHealth {
            subsystem: "Osc".into(),
            advisory: StepSizeAdvisory::Ok {
                ticks_per_cycle: 40,
            },
        };
        assert!(ok.message("sess", 1.0).is_none());

        let na = SubsystemStepSizeHealth {
            subsystem: "Osc".into(),
            advisory: StepSizeAdvisory::NotApplicable,
        };
        assert!(na.message("sess", 1.0).is_none());

        let under = SubsystemStepSizeHealth {
            subsystem: "Osc".into(),
            advisory: StepSizeAdvisory::UnderResolved {
                ticks_per_cycle: 6,
                suggested_dt_ms: 0.3,
            },
        };
        let msg = under.message("abc123", 1.0).expect("under-resolved message");
        assert!(msg.contains("subsystem Osc"));
        assert!(msg.contains("Session abc123"));
        assert!(msg.contains("numerical step-size advisory"));
        // Never alarm vocabulary.
        assert!(!msg.to_lowercase().contains("error"));
        assert!(!msg.to_lowercase().contains("bug"));
    }
}
