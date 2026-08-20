//! # SysML v2 Clock Model (per Clocks.kerml)
//!
//! Provides a monotonically advancing `currentTime` reference for simulation.
//! The `universalClock` is the default singleton clock used by the orchestrator.
//!
//! ## Spec Foundation
//!
//! From KerML `Clocks.kerml`:
//! - `Clock` provides a monotonically advancing `currentTime`
//! - `BasicClock` specializes `Clock` with `Real` `currentTime`
//! - `TimeOf` returns the time of an occurrence's start relative to a clock
//! - `DurationOf` returns the duration of an occurrence
//! - `universalClock` is the default clock used when no explicit clock is given

use std::collections::HashMap;

/// A clock that tracks monotonically advancing time.
///
/// Mirrors the KerML `Clock` concept: a named source of `currentTime`
/// that advances discretely via [`advance`](Self::advance).
#[derive(Debug, Clone)]
pub struct Clock {
    /// Human-readable name (e.g., `"universalClock"`).
    pub name: String,
    /// Current time in seconds.
    pub current_time: f64,
}

impl Clock {
    /// Create a new clock starting at time zero.
    pub fn new(name: impl Into<String>) -> Self {
        Clock {
            name: name.into(),
            current_time: 0.0,
        }
    }

    /// Advance the clock by `dt` seconds.
    ///
    /// Per the spec, a Clock's `currentTime` advances monotonically.
    /// Negative `dt` is clamped to zero in release builds (debug asserts).
    pub fn advance(&mut self, dt: f64) {
        debug_assert!(dt >= 0.0, "Clock::advance called with negative dt={}", dt);
        self.current_time += dt.max(0.0);
    }

    /// Reset the clock to time zero.
    pub fn reset(&mut self) {
        self.current_time = 0.0;
    }
}

/// `BasicClock` specializes `Clock` with `Real` `currentTime`.
///
/// In our model this is identical to [`Clock`] since we already use `f64`.
pub type BasicClock = Clock;

/// Return the time of an occurrence's start relative to a clock.
///
/// **Simplification**: In the current model all clocks share a single time
/// reference, so the `clock` parameter is accepted for API compatibility
/// with the spec's `TimeOf(o, clock)` but not used for offset computation.
/// A future multi-rate simulation would compute time relative to each clock.
pub fn time_of(occurrence_start_time: f64, _clock: &Clock) -> f64 {
    occurrence_start_time
}

/// Return the duration of an occurrence (end minus start).
///
/// **Simplification**: The `clock` parameter is accepted for spec alignment
/// but not used — see [`time_of`] for rationale.
pub fn duration_of(start_time: f64, end_time: f64, _clock: &Clock) -> f64 {
    end_time - start_time
}

// ---------------------------------------------------------------------------
// Multi-rate clocks (Feature 10.1)
// ---------------------------------------------------------------------------

/// A local clock for a subsystem with its own rate.
///
/// When the global clock advances by dt, a local clock advances by dt * rate.
/// Rate 1.0 = real-time. Rate 2.0 = double speed. Rate 0.5 = half speed.
///
/// This mirrors the KerML `localClock` concept: each subsystem may perceive
/// time at a different rate relative to the universal clock.
#[derive(Debug, Clone)]
pub struct LocalClock {
    /// Clock name (typically matches subsystem name).
    pub name: String,
    /// Rate multiplier relative to the universal clock.
    pub rate: f64,
    /// Current local time in seconds.
    pub current_time: f64,
    /// Phase offset in seconds (initial time offset).
    pub phase_offset: f64,
}

impl LocalClock {
    /// Create a new local clock with the given rate multiplier.
    ///
    /// Starts at time zero with no phase offset.
    pub fn new(name: impl Into<String>, rate: f64) -> Self {
        Self {
            name: name.into(),
            rate,
            current_time: 0.0,
            phase_offset: 0.0,
        }
    }

    /// Set a phase offset (initial time offset) using the builder pattern.
    ///
    /// The clock's current time is also set to the offset so it starts there.
    pub fn with_phase_offset(mut self, offset: f64) -> Self {
        self.phase_offset = offset;
        self.current_time = offset;
        self
    }

    /// Advance local time by global_dt * rate.
    pub fn advance(&mut self, global_dt: f64) {
        self.current_time += global_dt * self.rate;
    }

    /// Reset to phase offset.
    pub fn reset(&mut self) {
        self.current_time = self.phase_offset;
    }
}

/// Registry mapping subsystem names to their local clocks.
///
/// The orchestrator holds a `ClockRegistry` and advances all local clocks
/// each tick. Subsystems without a registered local clock use the global
/// universal clock time.
#[derive(Debug, Clone, Default)]
pub struct ClockRegistry {
    clocks: HashMap<String, LocalClock>,
}

impl ClockRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a local clock for a subsystem.
    pub fn register(&mut self, clock: LocalClock) {
        self.clocks.insert(clock.name.clone(), clock);
    }

    /// Advance all local clocks by the given global time step.
    pub fn advance_all(&mut self, global_dt: f64) {
        for clock in self.clocks.values_mut() {
            clock.advance(global_dt);
        }
    }

    /// Get the local time for a subsystem.
    pub fn local_time(&self, subsystem: &str) -> Option<f64> {
        self.clocks.get(subsystem).map(|c| c.current_time)
    }

    /// Get the rate for a subsystem.
    pub fn rate(&self, subsystem: &str) -> Option<f64> {
        self.clocks.get(subsystem).map(|c| c.rate)
    }

    /// Get a reference to a local clock by subsystem name.
    pub fn get(&self, subsystem: &str) -> Option<&LocalClock> {
        self.clocks.get(subsystem)
    }

    /// Reset all clocks to their phase offsets.
    pub fn reset_all(&mut self) {
        for clock in self.clocks.values_mut() {
            clock.reset();
        }
    }

    /// Check if a subsystem has a registered local clock.
    pub fn contains(&self, subsystem: &str) -> bool {
        self.clocks.contains_key(subsystem)
    }

    /// Return the number of registered clocks.
    pub fn len(&self) -> usize {
        self.clocks.len()
    }

    /// Return whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.clocks.is_empty()
    }
}

/// Compute the time of an occurrence relative to a local clock.
///
/// Transforms a global occurrence time into the local clock's time domain.
pub fn time_of_with_clock(occurrence_time: f64, clock: &LocalClock) -> f64 {
    occurrence_time * clock.rate + clock.phase_offset
}

/// Compute the duration of an occurrence relative to a local clock.
///
/// The duration is scaled by the clock's rate: a 1-second global duration
/// becomes 2 seconds on a 2x clock and 0.5 seconds on a 0.5x clock.
pub fn duration_of_with_clock(start_time: f64, end_time: f64, clock: &LocalClock) -> f64 {
    (end_time - start_time) * clock.rate
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn clock_creation() {
        let c = Clock::new("test");
        assert_eq!(c.name, "test");
        assert!((c.current_time - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn clock_advance() {
        let mut c = Clock::new("t");
        c.advance(1.5);
        assert!((c.current_time - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn clock_multiple_advances_accumulate() {
        let mut c = Clock::new("t");
        c.advance(0.1);
        c.advance(0.2);
        c.advance(0.3);
        assert!((c.current_time - 0.6).abs() < 1e-12);
    }

    #[test]
    fn clock_reset() {
        let mut c = Clock::new("t");
        c.advance(5.0);
        assert!(c.current_time > 0.0);
        c.reset();
        assert!((c.current_time - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn clock_negative_dt_clamped() {
        // Negative dt is clamped to 0.0 (monotonicity per spec).
        // In debug builds this also fires a debug_assert.
        let mut c = Clock::new("t");
        c.advance(3.0);
        // In release: clamps to 0, time stays at 3.0
        // In debug: panics (tested separately below)
        #[cfg(not(debug_assertions))]
        {
            c.advance(-1.0);
            assert!((c.current_time - 3.0).abs() < f64::EPSILON);
        }
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "negative dt")]
    fn clock_negative_dt_panics_in_debug() {
        let mut c = Clock::new("t");
        c.advance(3.0);
        c.advance(-1.0); // should panic
    }

    #[test]
    fn time_of_returns_start_time() {
        let c = Clock::new("clk");
        assert!((time_of(4.5, &c) - 4.5).abs() < f64::EPSILON);
    }

    #[test]
    fn duration_of_computes_difference() {
        let c = Clock::new("clk");
        assert!((duration_of(1.0, 3.5, &c) - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn basic_clock_alias() {
        // BasicClock is a type alias for Clock.
        let c: BasicClock = BasicClock::new("basic");
        assert_eq!(c.name, "basic");
    }

    // -----------------------------------------------------------------------
    // LocalClock tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_local_clock_rate() {
        let mut clock = LocalClock::new("fast", 2.0);
        clock.advance(1.0); // 1s global = 2s local
        assert!((clock.current_time - 2.0).abs() < 1e-10);
    }

    #[test]
    fn test_local_clock_half_speed() {
        let mut clock = LocalClock::new("slow", 0.5);
        clock.advance(4.0); // 4s global = 2s local
        assert!((clock.current_time - 2.0).abs() < 1e-10);
    }

    #[test]
    fn test_local_clock_phase_offset() {
        let mut clock = LocalClock::new("delayed", 1.0).with_phase_offset(5.0);
        assert!((clock.current_time - 5.0).abs() < 1e-10);
        clock.advance(3.0);
        assert!((clock.current_time - 8.0).abs() < 1e-10);
    }

    #[test]
    fn test_local_clock_reset_to_phase_offset() {
        let mut clock = LocalClock::new("offset", 1.0).with_phase_offset(10.0);
        clock.advance(5.0);
        assert!((clock.current_time - 15.0).abs() < 1e-10);
        clock.reset();
        assert!((clock.current_time - 10.0).abs() < 1e-10);
    }

    #[test]
    fn test_local_clock_reset_no_offset() {
        let mut clock = LocalClock::new("plain", 3.0);
        clock.advance(2.0);
        assert!((clock.current_time - 6.0).abs() < 1e-10);
        clock.reset();
        assert!((clock.current_time - 0.0).abs() < 1e-10);
    }

    // -----------------------------------------------------------------------
    // ClockRegistry tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_clock_registry() {
        let mut reg = ClockRegistry::new();
        reg.register(LocalClock::new("controller", 10.0)); // 10x speed
        reg.register(LocalClock::new("thermal", 0.1)); // 0.1x speed
        reg.advance_all(1.0); // 1 second global
        assert!((reg.local_time("controller").unwrap() - 10.0).abs() < 1e-10);
        assert!((reg.local_time("thermal").unwrap() - 0.1).abs() < 1e-10);
    }

    #[test]
    fn test_clock_registry_reset() {
        let mut reg = ClockRegistry::new();
        reg.register(LocalClock::new("sub", 2.0));
        reg.advance_all(5.0);
        assert!((reg.local_time("sub").unwrap() - 10.0).abs() < 1e-10);
        reg.reset_all();
        assert!((reg.local_time("sub").unwrap() - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_clock_registry_reset_with_phase_offset() {
        let mut reg = ClockRegistry::new();
        reg.register(LocalClock::new("sub", 1.0).with_phase_offset(3.0));
        reg.advance_all(5.0);
        assert!((reg.local_time("sub").unwrap() - 8.0).abs() < 1e-10);
        reg.reset_all();
        assert!((reg.local_time("sub").unwrap() - 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_clock_registry_unknown_subsystem() {
        let reg = ClockRegistry::new();
        assert!(reg.local_time("nonexistent").is_none());
        assert!(reg.rate("nonexistent").is_none());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn test_clock_registry_rate() {
        let mut reg = ClockRegistry::new();
        reg.register(LocalClock::new("fast", 5.0));
        assert!((reg.rate("fast").unwrap() - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_clock_registry_contains_and_len() {
        let mut reg = ClockRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        reg.register(LocalClock::new("a", 1.0));
        reg.register(LocalClock::new("b", 2.0));
        assert!(!reg.is_empty());
        assert_eq!(reg.len(), 2);
        assert!(reg.contains("a"));
        assert!(!reg.contains("c"));
    }

    // -----------------------------------------------------------------------
    // time_of_with_clock / duration_of_with_clock tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_time_of_with_clock() {
        let clock = LocalClock::new("fast", 2.0);
        // occurrence at global t=3.0 → 3.0 * 2.0 + 0.0 = 6.0
        assert!((time_of_with_clock(3.0, &clock) - 6.0).abs() < 1e-10);
    }

    #[test]
    fn test_time_of_with_clock_phase_offset() {
        let clock = LocalClock::new("offset", 1.0).with_phase_offset(5.0);
        // occurrence at global t=3.0 → 3.0 * 1.0 + 5.0 = 8.0
        assert!((time_of_with_clock(3.0, &clock) - 8.0).abs() < 1e-10);
    }

    #[test]
    fn test_duration_of_with_clock() {
        let clock = LocalClock::new("fast", 2.0);
        // duration 1.0→3.5 global → (3.5 - 1.0) * 2.0 = 5.0
        assert!((duration_of_with_clock(1.0, 3.5, &clock) - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_duration_of_with_clock_half_speed() {
        let clock = LocalClock::new("slow", 0.5);
        // duration 0.0→4.0 global → 4.0 * 0.5 = 2.0
        assert!((duration_of_with_clock(0.0, 4.0, &clock) - 2.0).abs() < 1e-10);
    }
}
