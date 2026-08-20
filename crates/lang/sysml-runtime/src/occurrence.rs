//! Occurrence lifecycle tracking for simulation events.
//!
//! Tracks when occurrences (state executions, action executions, flow transfers)
//! start and end, computing duration and capturing system state at boundaries.

use std::collections::{HashMap, VecDeque};
use sysml_core::Value;

/// The kind of occurrence being tracked.
#[derive(Debug, Clone, PartialEq)]
pub enum OccurrenceKind {
    /// A state machine state execution (entry -> exit).
    StateExecution,
    /// An action node execution (token enters -> leaves).
    ActionExecution,
    /// A flow transfer between ports.
    FlowTransfer,
    /// A generic event occurrence.
    EventOccurrence,
}

/// A tracked occurrence with start/end timing and captured features.
#[derive(Debug, Clone)]
pub struct Occurrence {
    /// Unique name/id for this occurrence.
    pub name: String,
    /// Kind of occurrence.
    pub kind: OccurrenceKind,
    /// Subsystem this occurrence belongs to.
    pub subsystem: String,
    /// Start time in seconds.
    pub start_time: f64,
    /// End time in seconds (None if still active).
    pub end_time: Option<f64>,
    /// Duration in seconds (computed on end).
    pub duration: Option<f64>,
    /// System state captured at start.
    pub start_features: HashMap<String, Value>,
    /// System state captured at end.
    pub end_features: HashMap<String, Value>,
}

impl Occurrence {
    /// Check if this occurrence is still active (not yet ended).
    pub fn is_active(&self) -> bool {
        self.end_time.is_none()
    }
}

/// Default cap on retained completed occurrences per tracker.
///
/// Mirrors [`crate::causation::CausationRecorder`]'s ring-buffer strategy.
/// `completed` used to grow unbounded (~180 KB/tick on large multi-subsystem
/// workloads) and was deep-cloned into every rewind-archive slot,
/// which OOM-crashed the host. Oldest entries are now evicted once
/// `completed` exceeds this length; override via
/// [`OccurrenceTracker::with_capacity`] / [`OccurrenceTracker::set_max_completed`].
pub const DEFAULT_MAX_COMPLETED: usize = 512;

/// Tracks active and completed occurrences during simulation.
#[derive(Debug, Clone)]
pub struct OccurrenceTracker {
    /// Currently active occurrences (keyed by "subsystem:name").
    active: HashMap<String, Occurrence>,
    /// Completed occurrences in chronological order (oldest first).
    /// Bounded ring buffer — see `max_completed`.
    completed: VecDeque<Occurrence>,
    /// Maximum retained `completed` length. Oldest entries are evicted
    /// on overflow.
    max_completed: usize,
}

impl Default for OccurrenceTracker {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_MAX_COMPLETED)
    }
}

impl OccurrenceTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a tracker with an explicit cap on retained completed
    /// occurrences. Oldest entries are evicted once `completed` exceeds
    /// this length.
    pub fn with_capacity(max_completed: usize) -> Self {
        Self {
            active: HashMap::new(),
            completed: VecDeque::new(),
            max_completed,
        }
    }

    /// Change the cap on retained completed occurrences. If the tracker
    /// already holds more than `max_completed` entries, evicts the
    /// oldest immediately.
    pub fn set_max_completed(&mut self, max_completed: usize) {
        self.max_completed = max_completed;
        while self.completed.len() > self.max_completed {
            self.completed.pop_front();
        }
    }

    /// Push a newly completed occurrence, evicting the oldest entry if
    /// the tracker is at capacity.
    fn push_completed(&mut self, occ: Occurrence) {
        self.completed.push_back(occ);
        while self.completed.len() > self.max_completed {
            self.completed.pop_front();
        }
    }

    /// Begin tracking a new occurrence.
    pub fn begin(
        &mut self,
        kind: OccurrenceKind,
        subsystem: impl Into<String>,
        name: impl Into<String>,
        time: f64,
        features: HashMap<String, Value>,
    ) {
        let subsystem = subsystem.into();
        let name = name.into();
        let key = format!("{}:{}", subsystem, name);

        // If there's already an active occurrence for this key, end it first
        if let Some(mut prev) = self.active.remove(&key) {
            prev.end_time = Some(time);
            prev.duration = Some(time - prev.start_time);
            self.push_completed(prev);
        }

        self.active.insert(
            key,
            Occurrence {
                name,
                kind,
                subsystem,
                start_time: time,
                end_time: None,
                duration: None,
                start_features: features,
                end_features: HashMap::new(),
            },
        );
    }

    /// End an active occurrence.
    pub fn end(
        &mut self,
        subsystem: &str,
        name: &str,
        time: f64,
        features: HashMap<String, Value>,
    ) -> Option<&Occurrence> {
        let key = format!("{}:{}", subsystem, name);
        if let Some(mut occ) = self.active.remove(&key) {
            occ.end_time = Some(time);
            occ.duration = Some(time - occ.start_time);
            occ.end_features = features;
            self.push_completed(occ);
            self.completed.back()
        } else {
            None
        }
    }

    /// Get all currently active occurrences.
    pub fn active(&self) -> Vec<&Occurrence> {
        self.active.values().collect()
    }

    /// Get all completed occurrences (oldest first), bounded by the
    /// tracker's cap (see [`DEFAULT_MAX_COMPLETED`] /
    /// [`OccurrenceTracker::with_capacity`]).
    pub fn completed(&self) -> &VecDeque<Occurrence> {
        &self.completed
    }

    /// Get occurrences that overlap with the given time range.
    pub fn between(&self, t1: f64, t2: f64) -> Vec<&Occurrence> {
        self.completed
            .iter()
            .filter(|o| o.start_time <= t2 && o.end_time.unwrap_or(f64::MAX) >= t1)
            .collect()
    }

    /// Get the duration of a completed occurrence by name.
    pub fn duration_of(&self, subsystem: &str, name: &str) -> Option<f64> {
        self.completed
            .iter()
            .rev() // most recent first
            .find(|o| o.subsystem == subsystem && o.name == name)
            .and_then(|o| o.duration)
    }

    /// Get the most recent occurrence for a subsystem.
    pub fn latest(&self, subsystem: &str) -> Option<&Occurrence> {
        self.completed
            .iter()
            .rev()
            .find(|o| o.subsystem == subsystem)
    }

    /// Total number of tracked occurrences (active + completed).
    pub fn total_count(&self) -> usize {
        self.active.len() + self.completed.len()
    }

    /// Reset all tracking state.
    pub fn reset(&mut self) {
        self.active.clear();
        self.completed.clear();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_begin_end_occurrence() {
        let mut tracker = OccurrenceTracker::new();
        let mut features = HashMap::new();
        features.insert("temperature".into(), Value::Float(93.0));

        tracker.begin(
            OccurrenceKind::StateExecution,
            "boiler",
            "heating",
            1.0,
            features,
        );
        assert_eq!(tracker.active().len(), 1);
        assert_eq!(tracker.completed().len(), 0);

        let mut end_features = HashMap::new();
        end_features.insert("temperature".into(), Value::Float(95.0));
        tracker.end("boiler", "heating", 5.0, end_features);

        assert_eq!(tracker.active().len(), 0);
        assert_eq!(tracker.completed().len(), 1);

        let occ = &tracker.completed()[0];
        assert_eq!(occ.name, "heating");
        assert_eq!(occ.subsystem, "boiler");
        assert!((occ.duration.unwrap() - 4.0).abs() < 1e-10);
        assert_eq!(
            occ.start_features.get("temperature"),
            Some(&Value::Float(93.0))
        );
        assert_eq!(
            occ.end_features.get("temperature"),
            Some(&Value::Float(95.0))
        );
    }

    #[test]
    fn test_duration_of() {
        let mut tracker = OccurrenceTracker::new();
        tracker.begin(
            OccurrenceKind::StateExecution,
            "pump",
            "running",
            0.0,
            HashMap::new(),
        );
        tracker.end("pump", "running", 10.0, HashMap::new());

        assert!((tracker.duration_of("pump", "running").unwrap() - 10.0).abs() < 1e-10);
        assert!(tracker.duration_of("pump", "nonexistent").is_none());
    }

    #[test]
    fn test_between_query() {
        let mut tracker = OccurrenceTracker::new();
        // Occ 1: 0-5s
        tracker.begin(
            OccurrenceKind::StateExecution,
            "sm",
            "idle",
            0.0,
            HashMap::new(),
        );
        tracker.end("sm", "idle", 5.0, HashMap::new());
        // Occ 2: 3-8s (overlaps with occ 1)
        tracker.begin(
            OccurrenceKind::ActionExecution,
            "action",
            "grind",
            3.0,
            HashMap::new(),
        );
        tracker.end("action", "grind", 8.0, HashMap::new());
        // Occ 3: 10-15s
        tracker.begin(
            OccurrenceKind::StateExecution,
            "sm",
            "brewing",
            10.0,
            HashMap::new(),
        );
        tracker.end("sm", "brewing", 15.0, HashMap::new());

        let overlap = tracker.between(4.0, 7.0);
        assert_eq!(overlap.len(), 2); // idle (0-5) and grind (3-8) overlap [4,7]

        let late = tracker.between(11.0, 14.0);
        assert_eq!(late.len(), 1); // only brewing
    }

    #[test]
    fn test_auto_end_on_rebegin() {
        let mut tracker = OccurrenceTracker::new();
        tracker.begin(
            OccurrenceKind::StateExecution,
            "sm",
            "idle",
            0.0,
            HashMap::new(),
        );
        // Begin same key again — should auto-end the previous
        tracker.begin(
            OccurrenceKind::StateExecution,
            "sm",
            "idle",
            3.0,
            HashMap::new(),
        );

        assert_eq!(tracker.active().len(), 1);
        assert_eq!(tracker.completed().len(), 1);
        assert!((tracker.completed()[0].duration.unwrap() - 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_reset() {
        let mut tracker = OccurrenceTracker::new();
        tracker.begin(
            OccurrenceKind::EventOccurrence,
            "sys",
            "event",
            0.0,
            HashMap::new(),
        );
        tracker.end("sys", "event", 1.0, HashMap::new());
        tracker.begin(
            OccurrenceKind::EventOccurrence,
            "sys",
            "event2",
            2.0,
            HashMap::new(),
        );

        tracker.reset();
        assert_eq!(tracker.active().len(), 0);
        assert_eq!(tracker.completed().len(), 0);
        assert_eq!(tracker.total_count(), 0);
    }

    #[test]
    fn test_latest() {
        let mut tracker = OccurrenceTracker::new();
        tracker.begin(
            OccurrenceKind::StateExecution,
            "sm",
            "a",
            0.0,
            HashMap::new(),
        );
        tracker.end("sm", "a", 1.0, HashMap::new());
        tracker.begin(
            OccurrenceKind::StateExecution,
            "sm",
            "b",
            1.0,
            HashMap::new(),
        );
        tracker.end("sm", "b", 3.0, HashMap::new());

        let latest = tracker.latest("sm").unwrap();
        assert_eq!(latest.name, "b");
    }

    #[test]
    fn test_multiple_subsystems() {
        let mut tracker = OccurrenceTracker::new();
        tracker.begin(
            OccurrenceKind::StateExecution,
            "boiler",
            "heating",
            0.0,
            HashMap::new(),
        );
        tracker.begin(
            OccurrenceKind::ActionExecution,
            "brew",
            "grinding",
            0.5,
            HashMap::new(),
        );
        tracker.begin(
            OccurrenceKind::FlowTransfer,
            "flow",
            "water_to_boiler",
            1.0,
            HashMap::new(),
        );

        assert_eq!(tracker.active().len(), 3);
        assert_eq!(tracker.total_count(), 3);
    }

    #[test]
    fn test_occurrence_kind() {
        let occ = Occurrence {
            name: "test".into(),
            kind: OccurrenceKind::StateExecution,
            subsystem: "sm".into(),
            start_time: 0.0,
            end_time: None,
            duration: None,
            start_features: HashMap::new(),
            end_features: HashMap::new(),
        };
        assert!(occ.is_active());
    }
}
