//! # Zero-Crossing Detection for Hybrid Simulation (Phase 15D)
//!
//! Detects when continuous ODE state crosses discrete event boundaries.
//! For example, when temperature crosses 90°C, a state machine transition
//! should fire at the precise crossing time.
//!
//! ## Spec Alignment
//!
//! `StateSpaceRepresentation.sysml` defines `ZeroCrossingEventDef` as
//! events that `ContinuousStateSpaceDynamics` may cause. We implement
//! this as event functions `g(t, y, ctx)` that change sign at crossing points.

#![allow(clippy::indexing_slicing)]
use std::sync::Arc;

use crate::expressions::EvalContext;

/// Direction of a zero crossing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CrossingDirection {
    /// g goes from negative to positive (rising edge).
    Rising,
    /// g goes from positive to negative (falling edge).
    Falling,
    /// Either direction.
    Either,
}

/// An event function `g(t, y, ctx) -> f64`.
///
/// A zero crossing occurs when `g` changes sign. The direction
/// determines which sign changes trigger the event.
///
/// Wrapped in `Arc` so the enclosing `ZeroCrossingDetector` can be cloned
/// (required for `Orchestrator::fork`).
pub type EventFn = Arc<dyn Fn(f64, &[f64], &EvalContext) -> f64 + Send + Sync>;

/// A detected zero-crossing event.
#[derive(Debug, Clone)]
pub struct CrossingEvent {
    /// Name of the event (e.g., transition name or guard description).
    pub name: String,
    /// Approximate time of the crossing.
    pub time: f64,
    /// Direction of the crossing.
    pub direction: CrossingDirection,
    /// Value of g at the crossing point (should be near zero).
    pub residual: f64,
}

/// Registered event function with metadata.
#[derive(Clone)]
struct RegisteredEvent {
    name: String,
    func: EventFn,
    direction: CrossingDirection,
}

/// Detects zero crossings in event functions during ODE integration.
#[derive(Clone)]
pub struct ZeroCrossingDetector {
    events: Vec<RegisteredEvent>,
    /// Previous values of each event function (for sign-change detection).
    prev_values: Vec<f64>,
    /// Bisection tolerance for locating crossing time.
    tolerance: f64,
    /// Maximum bisection iterations.
    max_iterations: usize,
}

impl Default for ZeroCrossingDetector {
    fn default() -> Self {
        ZeroCrossingDetector {
            events: Vec::new(),
            prev_values: Vec::new(),
            tolerance: 1e-6,
            max_iterations: 50,
        }
    }
}

impl ZeroCrossingDetector {
    /// Create a new detector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set bisection tolerance.
    pub fn with_tolerance(mut self, tol: f64) -> Self {
        self.tolerance = tol;
        self
    }

    /// Register an event function.
    pub fn add_event(
        &mut self,
        name: impl Into<String>,
        direction: CrossingDirection,
        func: EventFn,
    ) {
        self.events.push(RegisteredEvent {
            name: name.into(),
            func,
            direction,
        });
        self.prev_values.push(f64::NAN); // Will be initialized on first check
    }

    /// Initialize previous values at the current state.
    pub fn initialize(&mut self, t: f64, y: &[f64], ctx: &EvalContext) {
        for (i, event) in self.events.iter().enumerate() {
            if i < self.prev_values.len() {
                self.prev_values[i] = (event.func)(t, y, ctx);
            }
        }
    }

    /// Check for zero crossings between `t_start` (previous state) and `t_end`
    /// (current state after an ODE step).
    ///
    /// Returns detected crossings sorted by time.
    pub fn check(
        &mut self,
        t_start: f64,
        t_end: f64,
        y_start: &[f64],
        y_end: &[f64],
        ctx: &EvalContext,
    ) -> Vec<CrossingEvent> {
        let mut crossings = Vec::new();

        for (i, event) in self.events.iter().enumerate() {
            let g_start = if self.prev_values[i].is_nan() {
                (event.func)(t_start, y_start, ctx)
            } else {
                self.prev_values[i]
            };
            let g_end = (event.func)(t_end, y_end, ctx);

            if let Some(cr) =
                self.locate_crossing(event, t_start, t_end, g_start, g_end, y_start, y_end, ctx)
            {
                crossings.push(cr);
            }

            // Update previous value for next call
            self.prev_values[i] = g_end;
        }

        // Sort by crossing time
        crossings.sort_by(|a, b| {
            a.time
                .partial_cmp(&b.time)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        crossings
    }

    /// Shared per-event crossing location: detect a directional sign change on
    /// `[t_start, t_end]` and, if present, bisect to the crossing time and
    /// interpolate the residual. Extracted from [`check`](Self::check) so the
    /// sign-change + bisection definition lives in one place (RSC-4.3).
    #[allow(clippy::too_many_arguments)]
    fn locate_crossing(
        &self,
        event: &RegisteredEvent,
        t_start: f64,
        t_end: f64,
        g_start: f64,
        g_end: f64,
        y_start: &[f64],
        y_end: &[f64],
        ctx: &EvalContext,
    ) -> Option<CrossingEvent> {
        let sign_change = match event.direction {
            CrossingDirection::Rising => g_start < 0.0 && g_end >= 0.0,
            CrossingDirection::Falling => g_start >= 0.0 && g_end < 0.0,
            CrossingDirection::Either => g_start.signum() != g_end.signum() && g_start != 0.0,
        };
        if !sign_change {
            return None;
        }

        // Locate the crossing time using bisection.
        let crossing_time = self.bisect_crossing(
            t_start, t_end, g_start, g_end, y_start, y_end, ctx, &event.func,
        );

        // Interpolate y at the crossing time.
        let alpha = if (t_end - t_start).abs() > 1e-15 {
            (crossing_time - t_start) / (t_end - t_start)
        } else {
            0.5
        };
        let y_cross: Vec<f64> = y_start
            .iter()
            .zip(y_end.iter())
            .map(|(a, b)| a + alpha * (b - a))
            .collect();
        let residual = (event.func)(crossing_time, &y_cross, ctx);

        Some(CrossingEvent {
            name: event.name.clone(),
            time: crossing_time,
            direction: event.direction,
            residual,
        })
    }

    /// Bisect to find the approximate crossing time.
    ///
    /// Uses linear interpolation between (t_start, g_start) and (t_end, g_end)
    /// with refinement via the Illinois method.
    #[allow(clippy::too_many_arguments)]
    fn bisect_crossing(
        &self,
        t_start: f64,
        t_end: f64,
        g_start: f64,
        g_end: f64,
        y_start: &[f64],
        y_end: &[f64],
        ctx: &EvalContext,
        func: &EventFn,
    ) -> f64 {
        let mut lo = t_start;
        let mut hi = t_end;
        let mut g_lo = g_start;
        let mut g_hi = g_end;
        // Illinois anti-stall: track consecutive same-side retentions
        let mut lo_retained = 0_u32;
        let mut hi_retained = 0_u32;

        for _ in 0..self.max_iterations {
            if (hi - lo) < self.tolerance {
                break;
            }

            // Regula falsi with Illinois anti-stall modification.
            // When the same endpoint is retained twice, halve its function
            // value to prevent the iteration from stalling.
            let mid = if (g_hi - g_lo).abs() < 1e-30 {
                // Guard: g_lo ≈ g_hi would cause division by zero → plain bisection
                (lo + hi) / 2.0
            } else {
                let raw = lo - g_lo * (hi - lo) / (g_hi - g_lo);
                raw.clamp(lo + 1e-15, hi - 1e-15)
            };

            // Interpolate y at mid
            let alpha = (mid - t_start) / (t_end - t_start);
            let y_mid: Vec<f64> = y_start
                .iter()
                .zip(y_end.iter())
                .map(|(a, b)| a + alpha * (b - a))
                .collect();

            let g_mid = func(mid, &y_mid, ctx);

            if g_mid.abs() < self.tolerance * 0.1 {
                return mid;
            }

            if g_mid.signum() == g_lo.signum() {
                lo = mid;
                g_lo = g_mid;
                lo_retained = 0;
                hi_retained += 1;
                // Illinois: halve g_hi after 2 consecutive retentions
                if hi_retained >= 2 {
                    g_hi /= 2.0;
                }
            } else {
                hi = mid;
                g_hi = g_mid;
                hi_retained = 0;
                lo_retained += 1;
                if lo_retained >= 2 {
                    g_lo /= 2.0;
                }
            }
        }

        (lo + hi) / 2.0
    }

    /// Number of registered events.
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Reset previous values (call when ODE state is reset).
    pub fn reset(&mut self) {
        for v in &mut self.prev_values {
            *v = f64::NAN;
        }
    }
}

/// Build an event function from a guard expression like `temperature >= 90`.
///
/// Converts `lhs >= rhs` into `g(t, y, ctx) = lhs - rhs` so that
/// a zero crossing of `g` corresponds to the guard becoming true.
pub fn guard_to_event_fn(var_name: String, threshold: f64) -> EventFn {
    Arc::new(move |_t, _y, ctx| {
        let current = ctx
            .get(&var_name)
            .and_then(|v| match v {
                sysml_core::Value::Float(f) => Some(*f),
                sysml_core::Value::Int(i) => Some(*i as f64),
                _ => None,
            })
            .unwrap_or(0.0);
        current - threshold
    })
}

/// Polarity of a comparator edge feeding a [`DutyCycleTracker`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgePolarity {
    /// The square-wave signal crossed its UPPER threshold going up
    /// (the located `Rising` crossing of `signal >= +threshold`).
    Positive,
    /// The square-wave signal crossed its LOWER threshold going down
    /// (the located `Falling` crossing of `signal <= -threshold`).
    Negative,
}

/// Per-pulse duty-cycle asymmetry of a threshold-comparator-driven oscillator
/// — the fault-detection duty metric (WS-D Stage 2).
///
/// **SPEC-SILENT sanctioned extension** (steward-ruled). The SysML v2 / KerML
/// spec defines the *language* of models; it is silent on what simulation
/// *measurements* a tool may synthesize from execution state. This observable
/// sits in the same tooling category as time series (`timeseries.rs`),
/// aggregates (`aggregates.rs`), and the WS-A2 crossing-event counts — none of
/// which have language backing. It is NOT a model element and requires no model
/// annotation.
///
/// It reproduces the Basis firmware's pulse-wave sensor
/// (`drivers/sensors/rc_sensor.hpp` `GetDuty` + `OnData`, backed by
/// `platform/pulse_wave_sensor/pulse_wave_sensor.hpp`): a comparator on the
/// drive signal produces a square wave whose HIGH dwell (positive drive
/// half-cycle) and LOW dwell (negative half-cycle) become asymmetric under a
/// DC fault bias. The normalized asymmetry,
/// `duty = (2·(high − low) + (rise − fall)) / (rise + fall)` (∈ [−1,+1],
/// 0 = symmetric, NaN if |diff| ≥ sum), is the fault-bias signature.
///
/// The square wave is identified by the *signal* (steward Q2 ruling: option B),
/// reusing the WS-A2 comparator wiring — the symmetric `Rising`/`Falling`
/// located-crossing pair on the ODE's drive signal. Edges arrive as
/// [`CrossingEvent`]s already produced by the orchestrator's phase-1b detector;
/// no separate edge detection or extra ODE read is needed.
///
/// **Per-pulse capture, hardware-timer-faithful** (redesigned Jul 7 — see
/// module history below). The real sensor drives two independent
/// capture-compare timers, one reset on each comparator edge, whose four
/// capture registers (`high_pulse_width`/`low_pulse_width`/`rising_period`/
/// `falling_period` in `PulseWaveSamplePoint`) are each simply *overwritten*
/// by their next matching edge — no pairing, alternation check, or FIFO.
/// `Self::high`/`low`/`rise`/`fall` mirror that: each is written on exactly one
/// edge polarity, using whatever the *other* polarity's register currently
/// holds, with no ordering requirement between them:
///
/// - `Positive` edge at `t`: `fall = t − last_pos` (period since the previous
///   `Positive` edge), `high = t − last_neg` (dwell since the most recent
///   `Negative` edge), then `last_pos = t`.
/// - `Negative` edge at `t`: `rise = t − last_neg` (period since the previous
///   `Negative` edge), `low = t − last_pos` (dwell since the most recent
///   `Positive` edge), then `last_neg = t`.
///
/// A sample is attempted on every `Positive` edge once all four registers hold
/// a value (mirrors the original cadence — cycles complete on `Positive`
/// edges). Because each register always reflects the *latest* matching edge,
/// an arbitrary run of same-polarity edges (waveform distortion under a large
/// fault bias — RSC N_fault cross-level detectability islands, Jul 7) can
/// corrupt at most the interval spanning that run; the very next clean edge
/// recovers the correct pairing immediately — no multi-cycle desync, unlike
/// the strict `n_prev < p_prev < n_last < p_last` alternation invariant this
/// replaces.
///
/// **Period-window rejection** (`rc_sensor.hpp` `OnData` ~200-205): a sample
/// is accepted only if `rise` and `fall` both fall within the sensor's valid
/// oscillation-period band ([`MIN_VALID_PERIOD_S`], [`MAX_VALID_PERIOD_S`]) —
/// otherwise the firmware pushes NaN rather than computing `GetDuty`, and so
/// do we (`skipped_out_of_band`).
///
/// **Implemented, unit-tested, NOT enabled by default** (ledger L52,
/// `valid_period` wide open rather than to the firmware band. The legacy
/// fixture's oscillator measurably runs 16-21kHz under fault bias — outside the
/// firmware's 9.5-14.5kHz design band — so enabling the literal band rejects
/// nearly every cycle across the fault-calibration-relevant sweep, including
/// the previously-clean 1x column. That's a model-frequency calibration
/// question, director/steward-gated, not a tracker bug — re-enable via
/// `.with_valid_period_range(MIN_VALID_PERIOD_S, MAX_VALID_PERIOD_S)` once
/// L52 resolves. The firmware constants themselves stay literal in the
/// meantime; do not invent a model-specific band as a workaround.
#[derive(Debug, Clone)]
pub struct DutyCycleTracker {
    /// `event_name` of the `Positive` (upper-threshold, `Rising`) comparator
    /// crossing on the square-wave signal.
    positive_event: String,
    /// `event_name` of the `Negative` (lower-threshold, `Falling`) crossing.
    negative_event: String,
    /// Most recent `Positive` edge time — hardware analogue: the instant
    /// Timer A's reset counter last restarted. Plain overwrite, no history.
    last_pos: Option<f64>,
    /// Most recent `Negative` edge time (Timer B's reset instant).
    last_neg: Option<f64>,
    /// Per-pulse capture registers — see the struct doc for the exact
    /// edge-to-register mapping. Each is captured atomically by its owning
    /// edge and simply overwritten by the next ("last capture wins", the
    /// hardware capture-compare semantics this models). Held, not cleared,
    /// across a rejected sample — a later edge may still pair with a stale
    /// but valid partner, exactly like the always-on hardware timers.
    high: Option<f64>,
    low: Option<f64>,
    rise: Option<f64>,
    fall: Option<f64>,
    /// Valid oscillation-period band `rise`/`fall` must both fall inside
    /// (firmware period-window rejection). Defaults wide open —
    /// **not** [`MIN_VALID_PERIOD_S`]/[`MAX_VALID_PERIOD_S`] — pending ledger
    /// L52 (see the struct doc's "Period-window rejection" section);
    /// overridable via [`Self::with_valid_period_range`], used by this
    /// feature's own unit tests and available to re-enable the firmware band
    /// once L52 resolves. Not a model-facing config surface either way (the
    /// firmware has no equivalent knob).
    valid_period: (f64, f64),
    /// In-band cycles observed (a cycle is attempted on each `Positive` edge
    /// once all four registers hold a value), including transient ones.
    cycles_seen: usize,
    /// Startup transient cycles to discard before emitting (SPEC-SILENT default
    /// [`Self::DEFAULT_TRANSIENT_CYCLES`]). B is seeded off-equilibrium, so the
    /// first cycles settle before the duty stabilises.
    transient_cycles: usize,
    /// Most recently computed valid duty (held between cycles for time-series
    /// capture). `None` until the first post-transient cycle completes.
    duty: Option<f64>,
    /// Count of cycles whose `GetDuty` was non-finite (|diff| ≥ sum) and skipped
    /// — surfaced for diagnostics rather than poisoning `scalar_vars`.
    skipped_nonfinite: usize,
    /// Count of samples rejected by the period-window check (`rise` or `fall`
    /// outside `valid_period`) — the companion counter to
    /// [`Self::skipped_nonfinite`].
    skipped_out_of_band: usize,
}

/// Firmware oscillation-frequency band (`drivers/sensors/rc_sensor.hpp`
/// `kMinOscillationFrequency` = 9.5e3 Hz / `kMaxOscillationFrequency` =
/// 14.5e3 Hz), expressed as the corresponding period bounds — period =
/// 1/frequency, so the frequency *upper* bound gives the period *lower*
/// bound and vice versa. `Init()` derives the same bounds in hardware ticks
/// (`_min_oscillation_period_ticks`/`_max_oscillation_period_ticks`); these
/// are the same bounds in seconds, matching this tracker's `f64` event times.
/// The legacy fixture's nominal comparator oscillation (~10.9kHz, period ~91.7µs)
/// sits inside this band.
pub const MIN_VALID_PERIOD_S: f64 = 1.0 / 14_500.0;
/// See [`MIN_VALID_PERIOD_S`].
pub const MAX_VALID_PERIOD_S: f64 = 1.0 / 9_500.0;

impl DutyCycleTracker {
    /// Default startup transient discard (cycles).
    pub const DEFAULT_TRANSIENT_CYCLES: usize = 2;

    /// Build a tracker for a comparator square wave whose upper-threshold
    /// (`Rising`) crossing is `positive_event` and lower-threshold (`Falling`)
    /// crossing is `negative_event` (the WS-A2 located-crossing `event_name`s).
    pub fn new(positive_event: impl Into<String>, negative_event: impl Into<String>) -> Self {
        DutyCycleTracker {
            positive_event: positive_event.into(),
            negative_event: negative_event.into(),
            last_pos: None,
            last_neg: None,
            high: None,
            low: None,
            rise: None,
            fall: None,
            // Period-window rejection implemented per firmware (9.5-14.5kHz,
            // rc_sensor.hpp) but NOT enabled here: the model's oscillator
            // runs 16-21kHz under fault bias (out of the device band) —
            // enabling the literal band rejects every cycle across the
            // fault-calibration sweep. Blocked on the model-frequency
            // calibration question — see ledger L52
            // via `.with_valid_period_range(MIN_VALID_PERIOD_S,
            // MAX_VALID_PERIOD_S)` once that resolves.
            valid_period: (0.0, f64::INFINITY),
            cycles_seen: 0,
            transient_cycles: Self::DEFAULT_TRANSIENT_CYCLES,
            duty: None,
            skipped_nonfinite: 0,
            skipped_out_of_band: 0,
        }
    }

    /// Override the startup-transient discard count.
    pub fn with_transient_cycles(mut self, n: usize) -> Self {
        self.transient_cycles = n;
        self
    }

    /// Override the valid oscillation-period band (default firmware-derived
    /// [`MIN_VALID_PERIOD_S`]/[`MAX_VALID_PERIOD_S`]). Test-only — see the
    /// field doc on [`DutyCycleTracker::valid_period`].
    pub fn with_valid_period_range(mut self, min: f64, max: f64) -> Self {
        self.valid_period = (min, max);
        self
    }

    /// Feed a located crossing event. Only the two comparator events this
    /// tracker was built for are consumed; all others (safety crossings, other
    /// pairs) are ignored. Returns the freshly-computed duty when this edge
    /// completed a post-transient, in-band sample, else `None`.
    pub fn observe(&mut self, event_name: &str, time: f64) -> Option<f64> {
        let polarity = if event_name == self.positive_event {
            EdgePolarity::Positive
        } else if event_name == self.negative_event {
            EdgePolarity::Negative
        } else {
            return None;
        };

        match polarity {
            EdgePolarity::Negative => {
                if let Some(prev_neg) = self.last_neg {
                    self.rise = Some(time - prev_neg);
                }
                if let Some(last_pos) = self.last_pos {
                    self.low = Some(time - last_pos);
                }
                self.last_neg = Some(time);
                None
            }
            EdgePolarity::Positive => {
                if let Some(prev_pos) = self.last_pos {
                    self.fall = Some(time - prev_pos);
                }
                if let Some(last_neg) = self.last_neg {
                    self.high = Some(time - last_neg);
                }
                self.last_pos = Some(time);
                self.try_emit()
            }
        }
    }

    /// Attempt to close a sample from the current register contents. Called
    /// on every `Positive` edge (see [`Self::observe`]).
    fn try_emit(&mut self) -> Option<f64> {
        let (Some(high), Some(low), Some(rise), Some(fall)) =
            (self.high, self.low, self.rise, self.fall)
        else {
            return None;
        };

        let (min_period, max_period) = self.valid_period;
        if !(min_period..=max_period).contains(&rise) || !(min_period..=max_period).contains(&fall)
        {
            // Firmware period-window rejection (`rc_sensor.hpp` `OnData`): an
            // out-of-band period means the sensor isn't oscillating cleanly
            // this cycle (e.g. a comparator bounce shortened the interval) —
            // reject the whole sample, held registers included, rather than
            // feed a garbage `GetDuty` input.
            self.skipped_out_of_band += 1;
            return None;
        }

        self.cycles_seen += 1;
        if self.cycles_seen <= self.transient_cycles {
            return None;
        }

        let diff = 2.0 * (high - low) + (rise - fall);
        let sum = rise + fall;
        if sum > 0.0 && diff.abs() < sum {
            let d = diff / sum;
            self.duty = Some(d);
            Some(d)
        } else {
            // |diff| >= sum (out of band) → NaN in the firmware. Defensive
            // firmware parity: skip the write rather than poison
            // scalar_vars (fail-hard, Principle 2), holding the last valid
            // `duty`.
            self.skipped_nonfinite += 1;
            None
        }
    }

    /// The most recently computed valid duty, held between cycles for snapshot /
    /// time-series capture. `None` until the first post-transient cycle.
    pub fn current_duty(&self) -> Option<f64> {
        self.duty
    }

    /// Count of post-transient cycles whose duty was non-finite and skipped.
    pub fn skipped_nonfinite(&self) -> usize {
        self.skipped_nonfinite
    }

    /// Count of samples rejected by the period-window check. See the struct
    /// doc's "Period-window rejection" section.
    pub fn skipped_out_of_band(&self) -> usize {
        self.skipped_out_of_band
    }

    /// In-band cycles observed so far, including transient ones still being
    /// discarded. Lets tests/diagnostics distinguish "never samples" from
    /// "samples but stays in the transient window".
    pub fn cycles_seen(&self) -> usize {
        self.cycles_seen
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    /// Feed `n_cycles` of a square wave with positive-half width `high` and
    /// negative-half width `low` to a tracker, starting from a `Negative` edge
    /// at t=0. Returns the last duty emitted (post-transient).
    fn drive_duty(tracker: &mut DutyCycleTracker, high: f64, low: f64, n_cycles: usize) -> Option<f64> {
        // Edge order in time: N0, P0, N1, P1, ... ; high = P_k − N_k (asc half),
        // low = N_{k+1} − P_k (desc half). period = high + low.
        let mut t = 0.0;
        let mut last = None;
        // Prime with the first Negative edge.
        tracker.observe("neg", t);
        for _ in 0..n_cycles + 3 {
            t += high; // Positive edge `high` after the Negative edge.
            if let Some(d) = tracker.observe("pos", t) {
                last = Some(d);
            }
            t += low; // Next Negative edge `low` after the Positive edge.
            tracker.observe("neg", t);
        }
        last
    }

    #[test]
    fn test_duty_symmetric_is_zero() {
        let mut tracker = DutyCycleTracker::new("pos", "neg").with_transient_cycles(0);
        let d = drive_duty(&mut tracker, 5.0, 5.0, 4).expect("duty after symmetric cycles");
        assert!(d.abs() < 1e-9, "symmetric square wave → duty≈0, got {d}");
    }

    #[test]
    fn test_duty_asymmetric_sign_and_value() {
        // high=4, low=6 → duty = (high−low)/(high+low) = -0.2 (steady state).
        let mut tracker = DutyCycleTracker::new("pos", "neg").with_transient_cycles(0);
        let d = drive_duty(&mut tracker, 4.0, 6.0, 4).expect("duty");
        assert!((d - (-0.2)).abs() < 1e-9, "expected -0.2, got {d}");

        // high=6, low=4 → +0.2 (opposite bias).
        let mut t2 = DutyCycleTracker::new("pos", "neg").with_transient_cycles(0);
        let d2 = drive_duty(&mut t2, 6.0, 4.0, 4).expect("duty");
        assert!((d2 - 0.2).abs() < 1e-9, "expected +0.2, got {d2}");
    }

    #[test]
    fn test_duty_monotonic_in_asymmetry() {
        let mut prev = f64::NEG_INFINITY;
        for excess in [0.0, 0.5, 1.0, 1.5, 2.0] {
            // high grows, low shrinks, period fixed at 10.
            let high = 5.0 + excess;
            let low = 5.0 - excess;
            let mut tracker = DutyCycleTracker::new("pos", "neg").with_transient_cycles(0);
            let d = drive_duty(&mut tracker, high, low, 4).expect("duty");
            assert!(d > prev, "duty should increase with asymmetry: {d} !> {prev}");
            prev = d;
        }
    }

    #[test]
    fn test_duty_discards_transient() {
        // With 2 transient cycles, the first two *completed* cycles emit nothing.
        let mut tracker = DutyCycleTracker::new("pos", "neg"); // default transient = 2
        assert_eq!(tracker.current_duty(), None);
        let mut t = 0.0;
        tracker.observe("neg", t);
        t += 4.0;
        tracker.observe("pos", t); // first Positive: no prior cycle to close
        // Two transient cycles → still None.
        for i in 0..2 {
            t += 6.0;
            tracker.observe("neg", t);
            t += 4.0;
            assert_eq!(tracker.observe("pos", t), None, "transient cycle {i} emitted");
        }
        // The 3rd completed cycle (post-transient) emits.
        t += 6.0;
        tracker.observe("neg", t);
        t += 4.0;
        let d = tracker.observe("pos", t).expect("post-transient duty");
        assert!((d - (-0.2)).abs() < 1e-9, "got {d}");
    }

    #[test]
    fn test_duty_ignores_non_comparator_events() {
        let mut tracker = DutyCycleTracker::new("pos", "neg").with_transient_cycles(0);
        // Safety crossings on a different signal must not perturb the sequence.
        let mut t = 0.0;
        tracker.observe("neg", t);
        for _ in 0..6 {
            tracker.observe("__zc::safety::99", t + 0.1); // ignored
            t += 4.0;
            tracker.observe("pos", t);
            tracker.observe("__zc::safety::99", t + 0.1); // ignored
            t += 6.0;
            tracker.observe("neg", t);
        }
        let d = tracker.current_duty().expect("duty despite safety noise");
        assert!((d - (-0.2)).abs() < 1e-9, "got {d}");
    }

    #[test]
    fn test_duty_held_across_incomplete_window() {
        // Once a valid duty is computed it is held for snapshot capture across
        // subsequent edges that don't yet close a new cycle.
        let mut tracker = DutyCycleTracker::new("pos", "neg").with_transient_cycles(0);
        let valid = drive_duty(&mut tracker, 4.0, 6.0, 4).expect("valid duty");
        // A lone Negative edge starts the next cycle but emits nothing (a
        // `Negative` edge never attempts a sample — see `observe`).
        let emitted = tracker.observe("neg", 100_000.0);
        assert_eq!(emitted, None);
        assert_eq!(tracker.current_duty(), Some(valid), "last valid held");
    }

    #[test]
    fn test_duty_period_window_rejects_out_of_band_oscillation() {
        // The period-window band defaults DISABLED (ledger L52 — see the
        // struct doc) — opt in explicitly to exercise it. Firmware valid
        // band ≈ [69µs, 105µs] (9.5-14.5kHz). A 200µs period (5kHz, well
        // outside) must be rejected — the tracker never emits.
        let mut tracker = DutyCycleTracker::new("pos", "neg")
            .with_transient_cycles(0)
            .with_valid_period_range(MIN_VALID_PERIOD_S, MAX_VALID_PERIOD_S);
        let d = drive_duty(&mut tracker, 80e-6, 120e-6, 4);
        assert_eq!(d, None, "out-of-band period must be rejected, got {d:?}");
        assert!(tracker.skipped_out_of_band() > 0);
        assert_eq!(
            tracker.skipped_nonfinite(),
            0,
            "rejection must be attributed to the period window, not GetDuty"
        );
    }

    #[test]
    fn test_duty_period_window_accepts_in_band_oscillation() {
        // ~10.9kHz (period ≈91.7µs), inside [69,105]µs — the legacy fixture's
        // near-baseline/weak-bias oscillation frequency (NOT its frequency
        // under fault bias — see ledger L52, why this band isn't enabled by
        // default).
        let mut tracker = DutyCycleTracker::new("pos", "neg")
            .with_transient_cycles(0)
            .with_valid_period_range(MIN_VALID_PERIOD_S, MAX_VALID_PERIOD_S);
        let d = drive_duty(&mut tracker, 40e-6, 51.7e-6, 4).expect("in-band duty");
        assert!(d < 0.0, "high<low should bias duty negative, got {d}");
        assert_eq!(tracker.skipped_out_of_band(), 0);
    }

    #[test]
    fn test_duty_recovers_immediately_after_bounce_burst() {
        // The bug this redesign fixes (RSC N_fault cross-level detectability
        // islands, Jul 7): under waveform distortion the comparator can cross
        // multiple times on one side before the opposite side fires. The old
        // strict `n_prev < p_prev < n_last < p_last` alternation invariant
        // could stay desynced for multiple subsequent cycles once violated.
        // Per-pulse register-overwrite capture must instead recover on the
        // very next clean, unbounced edge — no lingering misalignment.
        let mut tracker = DutyCycleTracker::new("pos", "neg").with_transient_cycles(0);

        let mut t = 0.0_f64;
        tracker.observe("neg", t); // prime

        // Two clean warmup cycles (high=4, low=6 → duty=-0.2).
        for _ in 0..2 {
            t += 4.0;
            tracker.observe("pos", t);
            t += 6.0;
            tracker.observe("neg", t);
        }
        assert!((tracker.current_duty().unwrap() - (-0.2)).abs() < 1e-9);

        // Distortion: a burst of 3 extra Positive-edge bounces crammed
        // before the "real" edge (itself still a clean 4.0 after the last
        // clean Negative edge) — each corrupts that one sample, which is
        // fine and expected.
        for _ in 0..4 {
            t += 1.0;
            tracker.observe("pos", t);
        }
        t += 6.0;
        tracker.observe("neg", t);

        // The very next clean, unbounced Positive edge must already read
        // the correct steady-state duty again.
        t += 4.0;
        let d = tracker.observe("pos", t);
        assert!(
            (d.unwrap() - (-0.2)).abs() < 1e-9,
            "tracker did not recover immediately after the bounce burst: {d:?}"
        );
    }

    #[test]
    fn test_rising_crossing_detected() {
        let mut detector = ZeroCrossingDetector::new();
        detector.add_event(
            "temp_crosses_90",
            CrossingDirection::Rising,
            Arc::new(|_t, y, _ctx| y[0] - 90.0),
        );

        let ctx = EvalContext::new();
        let y_start = vec![85.0]; // below threshold
        let y_end = vec![95.0]; // above threshold

        detector.initialize(0.0, &y_start, &ctx);
        let crossings = detector.check(0.0, 1.0, &y_start, &y_end, &ctx);

        assert_eq!(crossings.len(), 1);
        assert_eq!(crossings[0].name, "temp_crosses_90");
        // Crossing should be near t=0.5 (linear interpolation: 85 + 0.5*10 = 90)
        assert!(
            (crossings[0].time - 0.5).abs() < 0.01,
            "time={}",
            crossings[0].time
        );
    }

    #[test]
    fn test_falling_crossing_detected() {
        let mut detector = ZeroCrossingDetector::new();
        detector.add_event(
            "temp_drops_below_50",
            CrossingDirection::Falling,
            Arc::new(|_t, y, _ctx| y[0] - 50.0),
        );

        let ctx = EvalContext::new();
        let y_start = vec![60.0];
        let y_end = vec![40.0];

        detector.initialize(0.0, &y_start, &ctx);
        let crossings = detector.check(0.0, 1.0, &y_start, &y_end, &ctx);

        assert_eq!(crossings.len(), 1);
        assert_eq!(crossings[0].direction, CrossingDirection::Falling);
    }

    #[test]
    fn test_no_crossing_when_same_side() {
        let mut detector = ZeroCrossingDetector::new();
        detector.add_event(
            "temp_crosses_90",
            CrossingDirection::Rising,
            Arc::new(|_t, y, _ctx| y[0] - 90.0),
        );

        let ctx = EvalContext::new();
        let y_start = vec![80.0];
        let y_end = vec![85.0]; // still below 90

        detector.initialize(0.0, &y_start, &ctx);
        let crossings = detector.check(0.0, 1.0, &y_start, &y_end, &ctx);

        assert!(crossings.is_empty());
    }

    #[test]
    fn test_wrong_direction_ignored() {
        let mut detector = ZeroCrossingDetector::new();
        detector.add_event(
            "rising_only",
            CrossingDirection::Rising,
            Arc::new(|_t, y, _ctx| y[0] - 50.0),
        );

        let ctx = EvalContext::new();
        // Falling crossing should NOT trigger a Rising event
        let y_start = vec![60.0];
        let y_end = vec![40.0];

        detector.initialize(0.0, &y_start, &ctx);
        let crossings = detector.check(0.0, 1.0, &y_start, &y_end, &ctx);

        assert!(crossings.is_empty());
    }

    #[test]
    fn test_multiple_events() {
        let mut detector = ZeroCrossingDetector::new();
        detector.add_event(
            "cross_50",
            CrossingDirection::Either,
            Arc::new(|_t, y, _ctx| y[0] - 50.0),
        );
        detector.add_event(
            "cross_80",
            CrossingDirection::Either,
            Arc::new(|_t, y, _ctx| y[0] - 80.0),
        );

        let ctx = EvalContext::new();
        let y_start = vec![40.0];
        let y_end = vec![90.0]; // crosses both 50 and 80

        detector.initialize(0.0, &y_start, &ctx);
        let crossings = detector.check(0.0, 1.0, &y_start, &y_end, &ctx);

        assert_eq!(crossings.len(), 2);
        // First crossing (50) should come before second (80)
        assert!(crossings[0].time < crossings[1].time);
    }

    #[test]
    fn test_crossing_precision() {
        let mut detector = ZeroCrossingDetector::new().with_tolerance(1e-10);
        detector.add_event(
            "exact",
            CrossingDirection::Rising,
            Arc::new(|_t, y, _ctx| y[0] - 100.0),
        );

        let ctx = EvalContext::new();
        let y_start = vec![0.0];
        let y_end = vec![200.0];

        detector.initialize(0.0, &y_start, &ctx);
        let crossings = detector.check(0.0, 1.0, &y_start, &y_end, &ctx);

        assert_eq!(crossings.len(), 1);
        // Crossing at y=100 => t=0.5 with linear interpolation
        assert!(
            (crossings[0].time - 0.5).abs() < 1e-8,
            "expected 0.5, got {}",
            crossings[0].time
        );
    }

    #[test]
    fn test_guard_to_event_fn() {
        let func = guard_to_event_fn("temperature".to_string(), 90.0);
        let mut ctx = EvalContext::new();

        ctx.set("temperature".to_string(), sysml_core::Value::Float(85.0));
        assert!((func)(0.0, &[], &ctx) < 0.0); // below threshold

        ctx.set("temperature".to_string(), sysml_core::Value::Float(95.0));
        assert!((func)(0.0, &[], &ctx) > 0.0); // above threshold

        ctx.set("temperature".to_string(), sysml_core::Value::Float(90.0));
        assert!((func)(0.0, &[], &ctx).abs() < 1e-15); // at threshold
    }

    #[test]
    fn test_reset() {
        let mut detector = ZeroCrossingDetector::new();
        detector.add_event(
            "test",
            CrossingDirection::Rising,
            Arc::new(|_t, y, _ctx| y[0]),
        );

        let ctx = EvalContext::new();
        detector.initialize(0.0, &[1.0], &ctx);

        // After reset, prev values should be NaN (re-initialized on next check)
        detector.reset();
        // A check from negative to positive should trigger even though
        // the previous "initialized" value was positive
        let crossings = detector.check(0.0, 1.0, &[-1.0], &[1.0], &ctx);
        assert_eq!(crossings.len(), 1);
    }

    #[test]
    fn test_bisect_equal_g_values_no_nan() {
        // When g_lo == g_hi, the regula falsi denominator is zero.
        // The bisection fallback should prevent NaN.
        let mut detector = ZeroCrossingDetector::new();
        detector.add_event(
            "tricky",
            CrossingDirection::Either,
            // g(t, y) = y[0] - 50 but y is interpolated linearly,
            // so g_start and g_end can be very close
            Arc::new(|_t, y, _ctx| y[0] - 50.0),
        );

        let ctx = EvalContext::new();
        // Start and end with nearly identical g values but opposite signs
        // (achieved by having y cross 50 with very small margin)
        let y_start = vec![49.9999999999];
        let y_end = vec![50.0000000001];

        detector.initialize(0.0, &y_start, &ctx);
        let crossings = detector.check(0.0, 1.0, &y_start, &y_end, &ctx);

        // Should detect crossing without NaN
        assert_eq!(crossings.len(), 1);
        assert!(
            crossings[0].time.is_finite(),
            "crossing time must be finite, not NaN"
        );
        assert!(crossings[0].time >= 0.0 && crossings[0].time <= 1.0);
    }
}
