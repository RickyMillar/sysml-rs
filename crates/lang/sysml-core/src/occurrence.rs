//! Occurrence model types for SysML v2 spec compliance.
//!
//! Implements the formal occurrence lifecycle from `OccurrenceFunctions.kerml`:
//! `create`, `destroy`, `addNew`, `addNewAt`, `isDuring`.
//!
//! These types model the spec's `Occurrence`, `Life`, and temporal relationships
//! (`HappensDuring`, `HappensBefore`). They live in sysml-core because they are
//! spec-level semantic concepts, not runtime-specific.

use std::collections::HashMap;

use crate::{ElementId, Value};

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// A Life provides continuous identity for occurrences across time slices.
///
/// From `Occurrences.kerml`: an occurrence is a `portionOfLife` — multiple
/// occurrences can share a Life (portions relationship).
#[derive(Debug, Clone, PartialEq)]
pub struct Life {
    pub id: ElementId,
    pub name: Option<String>,
}

/// Temporal boundary of an occurrence (a zero-duration snapshot).
///
/// Captures the system state at a specific point in time.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub time: f64,
    pub features: HashMap<String, Value>,
}

/// An occurrence instance with lifecycle state.
///
/// Corresponds to the spec's `Occurrence` with `startShot`/`endShot` boundaries,
/// a `portionOfLife` reference, and suboccurrence containment.
#[derive(Debug, Clone)]
pub struct OccurrenceInstance {
    pub id: ElementId,
    /// The Life this occurrence is a portion of.
    pub life_id: ElementId,
    /// Temporal start boundary (set by `create`).
    pub start_shot: Option<Snapshot>,
    /// Temporal end boundary (set by `destroy`).
    pub end_shot: Option<Snapshot>,
    /// Name of the local clock driving this occurrence.
    pub local_clock: String,
    /// IDs of suboccurrences contained within this one.
    pub suboccurrences: Vec<ElementId>,
}

impl OccurrenceInstance {
    /// True if this occurrence has been created (has a start shot).
    pub fn is_created(&self) -> bool {
        self.start_shot.is_some()
    }

    /// True if this occurrence has been destroyed (has an end shot).
    pub fn is_destroyed(&self) -> bool {
        self.end_shot.is_some()
    }

    /// True if the occurrence is currently active (created but not destroyed).
    pub fn is_active(&self) -> bool {
        self.is_created() && !self.is_destroyed()
    }

    /// Start time, if created.
    pub fn start_time(&self) -> Option<f64> {
        self.start_shot.as_ref().map(|s| s.time)
    }

    /// End time, if destroyed.
    pub fn end_time(&self) -> Option<f64> {
        self.end_shot.as_ref().map(|s| s.time)
    }
}

/// Temporal relationship between occurrences.
///
/// From `Occurrences.kerml`:
/// - `HappensDuring`: A.start >= B.start AND A.end <= B.end
/// - `HappensBefore`: A.end <= B.start
#[derive(Debug, Clone, PartialEq)]
pub enum TemporalRelation {
    /// The `inner` occurrence happens during the `outer` occurrence.
    HappensDuring { inner: ElementId, outer: ElementId },
    /// The `before` occurrence ends before the `after` occurrence starts.
    HappensBefore { before: ElementId, after: ElementId },
}

// ---------------------------------------------------------------------------
// OccurrenceRegistry
// ---------------------------------------------------------------------------

/// Registry tracking all occurrence instances, lives, and temporal relationships.
///
/// This is the central data structure for the spec's occurrence lifecycle model.
/// Runtime functions (`create`, `destroy`, `isDuring`, `addNew`, `addNewAt`)
/// operate on this registry.
#[derive(Debug, Clone, Default)]
pub struct OccurrenceRegistry {
    instances: HashMap<ElementId, OccurrenceInstance>,
    lives: HashMap<ElementId, Life>,
    relations: Vec<TemporalRelation>,
    /// Counter for generating unique IDs.
    next_id: u64,
}

impl OccurrenceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Generate a fresh ElementId for a new occurrence or life.
    fn fresh_id(&mut self) -> ElementId {
        let id = ElementId::from_string(format!("__occ_{}", self.next_id));
        self.next_id += 1;
        id
    }

    /// Register a new Life and return its ID.
    pub fn create_life(&mut self, name: Option<String>) -> ElementId {
        let id = self.fresh_id();
        self.lives.insert(
            id.clone(),
            Life {
                id: id.clone(),
                name,
            },
        );
        id
    }

    /// Create a new occurrence: establishes its startShot at the given time.
    ///
    /// From `OccurrenceFunctions.kerml`: `create(occurrences)` — each occurrence's
    /// startShot HappensDuring the function call.
    ///
    /// Returns the new occurrence's ID.
    pub fn create(
        &mut self,
        life_id: ElementId,
        time: f64,
        features: HashMap<String, Value>,
        clock: String,
    ) -> ElementId {
        let id = self.fresh_id();
        // Auto-create life if it doesn't exist
        if !self.lives.contains_key(&life_id) {
            self.lives.insert(
                life_id.clone(),
                Life {
                    id: life_id.clone(),
                    name: None,
                },
            );
        }
        self.instances.insert(
            id.clone(),
            OccurrenceInstance {
                id: id.clone(),
                life_id,
                start_shot: Some(Snapshot { time, features }),
                end_shot: None,
                local_clock: clock,
                suboccurrences: Vec::new(),
            },
        );
        id
    }

    /// Destroy an occurrence: establishes its endShot at the given time.
    ///
    /// From `OccurrenceFunctions.kerml`: `destroy(occurrences)` — finalizes
    /// endShot during the function's performance.
    ///
    /// Returns true if the occurrence was found and destroyed.
    pub fn destroy(
        &mut self,
        occurrence_id: &ElementId,
        time: f64,
        features: HashMap<String, Value>,
    ) -> bool {
        if let Some(occ) = self.instances.get_mut(occurrence_id) {
            if occ.end_shot.is_none() {
                occ.end_shot = Some(Snapshot { time, features });
                return true;
            }
        }
        false
    }

    /// Check if a given time falls during an occurrence's active interval.
    ///
    /// From `OccurrenceFunctions.kerml`: `isDuring(occurrence)` — returns true
    /// if a performance happens during the occurrence's time interval.
    ///
    /// Returns true if: start_time <= time <= end_time (or time >= start_time if not yet ended).
    pub fn is_during(&self, occurrence_id: &ElementId, time: f64) -> bool {
        if let Some(occ) = self.instances.get(occurrence_id) {
            let started = occ.start_shot.as_ref().map_or(false, |s| time >= s.time);
            let not_ended = occ.end_shot.as_ref().map_or(true, |e| time <= e.time);
            started && not_ended
        } else {
            false
        }
    }

    /// Check if occurrence A happens before occurrence B.
    ///
    /// A.end <= B.start (both must have the relevant shots).
    pub fn happens_before(&self, a: &ElementId, b: &ElementId) -> bool {
        let a_end = self.instances.get(a).and_then(|o| o.end_time());
        let b_start = self.instances.get(b).and_then(|o| o.start_time());
        match (a_end, b_start) {
            (Some(ae), Some(bs)) => ae <= bs,
            _ => false,
        }
    }

    /// Check if occurrence A happens during occurrence B.
    ///
    /// A.start >= B.start AND A.end <= B.end
    pub fn happens_during(&self, inner: &ElementId, outer: &ElementId) -> bool {
        let inner_occ = self.instances.get(inner);
        let outer_occ = self.instances.get(outer);
        match (inner_occ, outer_occ) {
            (Some(i), Some(o)) => {
                let i_start = i.start_time().unwrap_or(f64::MAX);
                let o_start = o.start_time().unwrap_or(f64::MAX);
                let i_end = i.end_time().unwrap_or(f64::MIN);
                let o_end = o.end_time().unwrap_or(f64::MIN);
                i_start >= o_start && i_end <= o_end
            }
            _ => false,
        }
    }

    /// Add a temporal relation to the registry.
    pub fn add_relation(&mut self, relation: TemporalRelation) {
        self.relations.push(relation);
    }

    /// Get all portions (occurrences) of a given Life.
    pub fn portions_of_life(&self, life_id: &ElementId) -> Vec<&OccurrenceInstance> {
        self.instances
            .values()
            .filter(|occ| &occ.life_id == life_id)
            .collect()
    }

    /// Get an occurrence instance by ID.
    pub fn get(&self, id: &ElementId) -> Option<&OccurrenceInstance> {
        self.instances.get(id)
    }

    /// Get a mutable reference to an occurrence instance.
    pub fn get_mut(&mut self, id: &ElementId) -> Option<&mut OccurrenceInstance> {
        self.instances.get_mut(id)
    }

    /// Get all active (created but not destroyed) occurrences.
    pub fn active_occurrences(&self) -> Vec<&OccurrenceInstance> {
        self.instances.values().filter(|o| o.is_active()).collect()
    }

    /// Get all registered temporal relations.
    pub fn relations(&self) -> &[TemporalRelation] {
        &self.relations
    }

    /// Total number of occurrence instances.
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Total number of lives.
    pub fn life_count(&self) -> usize {
        self.lives.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_query() {
        let mut reg = OccurrenceRegistry::new();
        let life_id = reg.create_life(Some("sensor_life".into()));

        let occ_id = reg.create(
            life_id.clone(),
            1.0,
            HashMap::from([("temp".into(), Value::Float(25.0))]),
            "clock1".into(),
        );

        let occ = reg.get(&occ_id).unwrap();
        assert!(occ.is_created());
        assert!(occ.is_active());
        assert!(!occ.is_destroyed());
        assert_eq!(occ.start_time(), Some(1.0));
        assert_eq!(occ.end_time(), None);
        assert_eq!(occ.life_id, life_id);
    }

    #[test]
    fn test_destroy() {
        let mut reg = OccurrenceRegistry::new();
        let life_id = reg.create_life(None);
        let occ_id = reg.create(life_id, 0.0, HashMap::new(), "default".into());

        assert!(reg.is_during(&occ_id, 5.0));

        let destroyed = reg.destroy(
            &occ_id,
            10.0,
            HashMap::from([("status".into(), Value::String("done".into()))]),
        );
        assert!(destroyed);

        let occ = reg.get(&occ_id).unwrap();
        assert!(occ.is_destroyed());
        assert!(!occ.is_active());
        assert_eq!(occ.end_time(), Some(10.0));

        // Cannot destroy twice
        assert!(!reg.destroy(&occ_id, 15.0, HashMap::new()));
    }

    #[test]
    fn test_is_during() {
        let mut reg = OccurrenceRegistry::new();
        let life_id = reg.create_life(None);
        let occ_id = reg.create(life_id, 2.0, HashMap::new(), "default".into());
        reg.destroy(&occ_id, 8.0, HashMap::new());

        // Before start
        assert!(!reg.is_during(&occ_id, 1.0));
        // At start
        assert!(reg.is_during(&occ_id, 2.0));
        // During
        assert!(reg.is_during(&occ_id, 5.0));
        // At end
        assert!(reg.is_during(&occ_id, 8.0));
        // After end
        assert!(!reg.is_during(&occ_id, 9.0));
        // Nonexistent occurrence
        assert!(!reg.is_during(&ElementId::from_string("nonexistent"), 5.0));
    }

    #[test]
    fn test_happens_before() {
        let mut reg = OccurrenceRegistry::new();
        let life_id = reg.create_life(None);

        let a = reg.create(life_id.clone(), 0.0, HashMap::new(), "default".into());
        reg.destroy(&a, 5.0, HashMap::new());

        let b = reg.create(life_id, 5.0, HashMap::new(), "default".into());
        reg.destroy(&b, 10.0, HashMap::new());

        assert!(reg.happens_before(&a, &b));
        assert!(!reg.happens_before(&b, &a));
    }

    #[test]
    fn test_happens_during() {
        let mut reg = OccurrenceRegistry::new();
        let life_id = reg.create_life(None);

        let outer = reg.create(life_id.clone(), 0.0, HashMap::new(), "default".into());
        reg.destroy(&outer, 10.0, HashMap::new());

        let inner = reg.create(life_id, 2.0, HashMap::new(), "default".into());
        reg.destroy(&inner, 8.0, HashMap::new());

        assert!(reg.happens_during(&inner, &outer));
        assert!(!reg.happens_during(&outer, &inner));
    }

    #[test]
    fn test_portions_of_life() {
        let mut reg = OccurrenceRegistry::new();
        let life_id = reg.create_life(Some("entity_life".into()));

        let _a = reg.create(life_id.clone(), 0.0, HashMap::new(), "default".into());
        let _b = reg.create(life_id.clone(), 5.0, HashMap::new(), "default".into());

        let portions = reg.portions_of_life(&life_id);
        assert_eq!(portions.len(), 2);
    }

    #[test]
    fn test_active_occurrences() {
        let mut reg = OccurrenceRegistry::new();
        let life_id = reg.create_life(None);

        let a = reg.create(life_id.clone(), 0.0, HashMap::new(), "default".into());
        let _b = reg.create(life_id, 1.0, HashMap::new(), "default".into());
        reg.destroy(&a, 5.0, HashMap::new());

        let active = reg.active_occurrences();
        assert_eq!(active.len(), 1);
    }

    #[test]
    fn test_suboccurrences() {
        let mut reg = OccurrenceRegistry::new();
        let life_id = reg.create_life(None);

        let parent = reg.create(life_id.clone(), 0.0, HashMap::new(), "default".into());
        let child = reg.create(life_id, 1.0, HashMap::new(), "default".into());

        // Add child as suboccurrence of parent
        reg.get_mut(&parent)
            .unwrap()
            .suboccurrences
            .push(child.clone());

        let parent_occ = reg.get(&parent).unwrap();
        assert_eq!(parent_occ.suboccurrences.len(), 1);
        assert_eq!(parent_occ.suboccurrences[0], child);
    }

    #[test]
    fn test_temporal_relations() {
        let mut reg = OccurrenceRegistry::new();
        let life_id = reg.create_life(None);

        let a = reg.create(life_id.clone(), 0.0, HashMap::new(), "default".into());
        let b = reg.create(life_id, 5.0, HashMap::new(), "default".into());

        reg.add_relation(TemporalRelation::HappensBefore {
            before: a,
            after: b,
        });

        assert_eq!(reg.relations().len(), 1);
    }

    #[test]
    fn test_auto_create_life() {
        let mut reg = OccurrenceRegistry::new();
        // Create occurrence with a life_id that doesn't exist yet — should auto-create
        let fake_life = ElementId::from_string("auto_life");
        let occ_id = reg.create(fake_life.clone(), 0.0, HashMap::new(), "default".into());

        assert_eq!(reg.life_count(), 1);
        let occ = reg.get(&occ_id).unwrap();
        assert_eq!(occ.life_id, fake_life);
    }

    #[test]
    fn test_counts() {
        let mut reg = OccurrenceRegistry::new();
        assert_eq!(reg.instance_count(), 0);
        assert_eq!(reg.life_count(), 0);

        let life = reg.create_life(None);
        assert_eq!(reg.life_count(), 1);

        reg.create(life, 0.0, HashMap::new(), "default".into());
        assert_eq!(reg.instance_count(), 1);
    }
}
