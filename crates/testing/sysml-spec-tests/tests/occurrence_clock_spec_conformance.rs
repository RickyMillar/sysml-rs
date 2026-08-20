//! OCC — Occurrence & Clock spec-conformance harness.
//!
//! Sibling to `runtime_spec_conformance.rs` (RSC-0.2 already gates clock
//! `currentTime` monotonicity + `TimeOf` start-shot snapshot semantics) and
//! `constraint_spec_conformance.rs`. This file gates the **UNGATED behavioral**
//! occurrence/clock obligations catalogued in
//! `spec-obligations/occurrences-clocks.md` — the rows the tracker marks
//! `UNGATED`:
//!
//!   - `durationof-end-minus-start`
//!   - `timeof-ordering-constraint`
//!   - `timeof-continuity-constraint`
//!   - `local-clock-defaults-to-universal-clock`
//!   - `occurrence-has-lifetime-extent`
//!   - `happens-just-before-no-intervening`
//!
//! It deliberately does NOT duplicate the rows already GATED-elsewhere
//! (`clock-currenttime-advances-monotonically`,
//! `timeof-returns-start-shot-time` — both in `runtime_spec_conformance.rs`).
//!
//! Convention (matches the sibling files): every test encodes ONE spec
//! obligation, names it with an `// OBL:` line whose id matches the tracker,
//! and carries a verdict marker on its own line:
//!
//!   - `// VERDICT: CONFORMS` — current behavior satisfies the obligation.
//!   - `// VERDICT: DIVERGES — <reason>` — asserts what the engine ACTUALLY
//!     does today, which differs from the spec; fails only on silent change.
//!   - `// VERDICT: UNIMPLEMENTED — <missing>` — the obligation has no engine
//!     surface; the test pins the absence.
//!
//! Spec sources (cited per obligation in the tracker, which is the authority):
//!   - KerML §9.2.4 *Occurrences*, §9.2.12 *Clocks/TimeOf/DurationOf*
//!     (`KerML-spec-r2025-04_REF.html`)
//!   - `sysml.library/.../Clocks.kerml` (`TimeOf`, `DurationOf`,
//!     `timeOrderingConstraint`, `timeContinuityConstraint`, `universalClock`)
//!   - `sysml.library/.../Occurrences.kerml` (`startShot`/`endShot`, lifetime)
//!
//! Harness rules (match the siblings): pure runtime only. Behavioral
//! obligations are gated at the stable runtime surfaces that exist today —
//! `sysml_runtime::occurrence::OccurrenceTracker` (begin/end lifetime
//! tracking), `sysml_runtime::clock` free functions, and
//! `Orchestrator::set_clock` / the `__clock_time` context scalar. Where a spec
//! construct (model-callable `TimeOf`/`DurationOf`) has NO runtime surface, the
//! test is marked UNIMPLEMENTED rather than faked. NO LSP, NO SysmlService, NO
//! production-code changes — this file measures.
//!
//! The summary test (`occurrence_matrix_summary`) self-scans this file via
//! `include_str!` and prints the CONFORMS / DIVERGES / UNIMPLEMENTED counts.

use std::collections::HashMap;
use sysml_runtime::clock::{duration_of, time_of, Clock};
use sysml_runtime::occurrence::{OccurrenceKind, OccurrenceTracker};
use sysml_runtime::orchestrator::{Orchestrator, OrchestratorConfig};
use sysml_runtime::statemachine::StateMachineRunner;
use sysml_runtime::{StateIR, StateMachineIR, TransitionIR};

// ---------------------------------------------------------------------------
// Harness helpers
// ---------------------------------------------------------------------------

/// Begin then immediately end a tracked occurrence spanning `[start, end]`.
///
/// Mirrors the `OccurrenceTracker` begin/end idiom from `occurrence.rs`'s own
/// colocated tests. Returns the tracker so the completed occurrence can be
/// observed.
fn run_occurrence(name: &str, start: f64, end: f64) -> OccurrenceTracker {
    let mut tracker = OccurrenceTracker::new();
    tracker.begin(
        OccurrenceKind::EventOccurrence,
        "sys",
        name,
        start,
        HashMap::new(),
    );
    tracker.end("sys", name, end, HashMap::new());
    tracker
}

/// Take a zero-duration *snapshot* occurrence at instant `t` (startShot ==
/// endShot). A snapshot is the time slice of zero duration used by the Clock
/// `timeFlowConstraint` (KerML §9.2.4.1; `Occurrences.kerml`).
fn run_occurrence_snapshot(name: &str, t: f64) -> OccurrenceTracker {
    run_occurrence(name, t, t)
}

// ===========================================================================
// OBL — occurrence-has-lifetime-extent
// "An occurrence has an extent in time (lifetime) from start to end."
// SysML §8.4.5; KerML §9.2.4. Gated via the runtime occurrence lifetime tracker
// (start_time .. end_time is the realized lifetime extent).
// ===========================================================================

#[test]
fn occ_occurrence_has_lifetime_extent() {
    // OBL: occurrence-has-lifetime-extent
    // VERDICT: CONFORMS
    let tracker = run_occurrence("heating", 2.0, 7.0);
    let occ = &tracker.completed()[0];
    assert!(
        (occ.start_time - 2.0).abs() < 1e-12,
        "lifetime starts at the begin time"
    );
    assert_eq!(
        occ.end_time,
        Some(7.0),
        "lifetime ends at the end time (extent is bounded, not open)"
    );
    // The extent (end − start) is a positive interval: the occurrence exists
    // over a stretch of time, not a single instant.
    assert!(
        occ.end_time.unwrap() > occ.start_time,
        "an occurrence's lifetime extent spans start → end"
    );
}

// ===========================================================================
// OBL — durationof-end-minus-start
// "DurationOf = TimeOf(endShot) − TimeOf(startShot)."
// KerML §9.2.12.2.5; Clocks.kerml `DurationOf`. Gated two ways: the realized
// occurrence-tracker duration, and the pure `clock::duration_of` formula.
// ===========================================================================

#[test]
fn occ_durationof_equals_end_minus_start() {
    // OBL: durationof-end-minus-start
    // VERDICT: CONFORMS
    let tracker = run_occurrence("pump_run", 1.0, 4.5);
    // The tracker computes duration = end − start exactly per the spec formula.
    assert!(
        (tracker.duration_of("sys", "pump_run").unwrap() - 3.5).abs() < 1e-12,
        "DurationOf an occurrence = end − start (KerML §9.2.12.2.5)"
    );
    // The standalone clock formula agrees: DurationOf(start, end) = end − start.
    let c = Clock::new("universalClock");
    assert!(
        (duration_of(1.0, 4.5, &c) - 3.5).abs() < 1e-12,
        "clock::duration_of realizes the same end−start formula"
    );
    // Cross-check the formula identity TimeOf(end) − TimeOf(start):
    assert!(
        (duration_of(1.0, 4.5, &c) - (time_of(4.5, &c) - time_of(1.0, &c))).abs() < 1e-12,
        "DurationOf == TimeOf(endShot) − TimeOf(startShot)"
    );
}

// ===========================================================================
// OBL — timeof-ordering-constraint
// "If A HappensBefore B, then TimeOf(A.endShot) ≤ TimeOf(B)."
// KerML §9.2.12.2.6 `timeOrderingConstraint`; Clocks.kerml.
//
// There is NO model-callable `TimeOf` in the expression stdlib
// (runtime_spec_conformance.rs `spec_clock_timeof_snapshot_semantics` already
// pins that as UNIMPLEMENTED). But the realized ordering invariant — a
// non-overlapping earlier-before-later pair satisfies end(A) ≤ start(B) — IS
// observable on the occurrence tracker, which records completed occurrences
// chronologically. We gate the ordering invariant over realized times.
// ===========================================================================

#[test]
fn occ_timeof_ordering_constraint_holds_for_sequential_occurrences() {
    // OBL: timeof-ordering-constraint
    // VERDICT: CONFORMS
    // A: [0,3], B: [3,6] — A HappensBefore B (B begins as A ends).
    let mut tracker = OccurrenceTracker::new();
    tracker.begin(OccurrenceKind::EventOccurrence, "sys", "A", 0.0, HashMap::new());
    tracker.end("sys", "A", 3.0, HashMap::new());
    tracker.begin(OccurrenceKind::EventOccurrence, "sys", "B", 3.0, HashMap::new());
    tracker.end("sys", "B", 6.0, HashMap::new());

    let completed = tracker.completed();
    let a = &completed[0];
    let b = &completed[1];
    // timeOrderingConstraint: TimeOf(A.endShot) ≤ TimeOf(B.startShot).
    assert!(
        a.end_time.unwrap() <= b.start_time,
        "HappensBefore ⇒ end(A) ≤ start(B) (timeOrderingConstraint, KerML §9.2.12.2.6)"
    );
}

// ===========================================================================
// OBL — timeof-continuity-constraint
// "If A HappensJustBefore B, then TimeOf(A.endShot) == TimeOf(B)."
// KerML §9.2.12.2.6 `timeContinuityConstraint`; Clocks.kerml.
//
// Same surface caveat as ordering: no model-callable TimeOf. The realized
// continuity invariant (immediate succession ⇒ end(A) == start(B)) is gated
// over the occurrence tracker for an abutting pair.
// ===========================================================================

#[test]
fn occ_timeof_continuity_constraint_for_abutting_occurrences() {
    // OBL: timeof-continuity-constraint
    // VERDICT: CONFORMS
    // A: [0,5], B: [5,9] — B starts at the exact instant A ends (just-before).
    let mut tracker = OccurrenceTracker::new();
    tracker.begin(OccurrenceKind::EventOccurrence, "sys", "A", 0.0, HashMap::new());
    tracker.end("sys", "A", 5.0, HashMap::new());
    tracker.begin(OccurrenceKind::EventOccurrence, "sys", "B", 5.0, HashMap::new());
    tracker.end("sys", "B", 9.0, HashMap::new());

    let completed = tracker.completed();
    let a = &completed[0];
    let b = &completed[1];
    // timeContinuityConstraint: TimeOf(A.endShot) == TimeOf(B.startShot).
    assert!(
        (a.end_time.unwrap() - b.start_time).abs() < 1e-12,
        "HappensJustBefore ⇒ end(A) == start(B) (timeContinuityConstraint, KerML §9.2.12.2.6)"
    );
}

// ===========================================================================
// OBL — happens-just-before-no-intervening
// "HappensJustBefore: no occurrence can exist in the gap between earlier and
// later." KerML §9.2.4.2.4.
//
// The runtime has no `HappensJustBefore` relation type, but the immediate-
// succession PROPERTY is observable: for an abutting pair end(A) == start(B),
// the `between(end(A), start(B))` window is a zero-width point and contains no
// THIRD occurrence wholly inside the (empty) gap. We assert the gap is empty.
// ===========================================================================

#[test]
fn occ_happens_just_before_has_no_intervening_occurrence() {
    // OBL: happens-just-before-no-intervening
    // VERDICT: CONFORMS
    // A: [0,5], B: [5,10] abut. A distractor C: [20,25] is far in the future.
    let mut tracker = OccurrenceTracker::new();
    tracker.begin(OccurrenceKind::EventOccurrence, "sys", "A", 0.0, HashMap::new());
    tracker.end("sys", "A", 5.0, HashMap::new());
    tracker.begin(OccurrenceKind::EventOccurrence, "sys", "B", 5.0, HashMap::new());
    tracker.end("sys", "B", 10.0, HashMap::new());
    tracker.begin(OccurrenceKind::EventOccurrence, "sys", "C", 20.0, HashMap::new());
    tracker.end("sys", "C", 25.0, HashMap::new());

    let a_end = tracker.completed()[0].end_time.unwrap();
    let b_start = tracker.completed()[1].start_time;
    // The gap between A and B is the single instant [a_end, b_start] (== 5.0).
    // Occurrences overlapping that instant are only A and B themselves — there
    // is NO third (intervening) occurrence strictly between them.
    let in_gap = tracker.between(a_end, b_start);
    let names: Vec<&str> = in_gap.iter().map(|o| o.name.as_str()).collect();
    assert!(
        names.contains(&"A") && names.contains(&"B"),
        "the abutting pair touches its own boundary instant"
    );
    assert!(
        !names.contains(&"C"),
        "no occurrence intervenes between A (just-before) and B (KerML §9.2.4.2.4)"
    );
    assert_eq!(
        names.len(),
        2,
        "exactly A and B touch the boundary — the gap admits no intervening occurrence"
    );
}

// ===========================================================================
// OBL — local-clock-defaults-to-universal-clock
// "An occurrence's localClock defaults to Clocks::universalClock."
// KerML §9.2.4.2.13, §9.2.12.2.7.
//
// Runtime realization: a subsystem with NO registered local clock reads
// `__clock_time` from the universal clock (orchestrator.rs:2306 publishes the
// universal clock; per-subsystem local clocks override only when registered via
// set_clock). We gate the DEFAULT path: an unregistered subsystem's guard over
// __clock_time fires on universal-clock time.
// ===========================================================================

#[test]
fn occ_local_clock_defaults_to_universal_clock() {
    // OBL: local-clock-defaults-to-universal-clock
    // VERDICT: CONFORMS
    let ir = StateMachineIR {
        name: "default".to_string(),
        states: vec![StateIR::new("waiting"), StateIR::new("done").final_state()],
        transitions: vec![TransitionIR::new("waiting", "done")
            .with_event("check")
            .with_guard("__clock_time > 1.5".to_string())],
        initial: "waiting".to_string(),
        regions: vec![],
    };
    let mut orch = Orchestrator::new(OrchestratorConfig {
        dt_ms: 1000.0, // 1.0s per tick
        ..Default::default()
    });
    orch.add_state_machine("default", StateMachineRunner::new(ir));
    // No set_clock — the subsystem has no local clock and must default to the
    // universal clock (Clocks::universalClock).

    // After tick 1: universal clock = 1.0s, guard (> 1.5) fails.
    orch.inject_event("default", "check");
    let snap1 = orch.step();
    assert_eq!(
        snap1.subsystem_states["default"].current_state, "waiting",
        "default-clock subsystem reads universal-clock time (1.0s ≤ 1.5)"
    );
    // The registry has no local clock for this subsystem — the default path.
    assert!(
        orch.clock_registry().local_time("default").is_none(),
        "no local clock registered ⇒ fall back to universal clock"
    );

    // After tick 2: universal clock = 2.0s > 1.5, guard passes — proving the
    // guard saw universal-clock time, not a never-advancing default.
    orch.inject_event("default", "check");
    let snap2 = orch.step();
    assert_eq!(
        snap2.subsystem_states["default"].current_state, "done",
        "universal clock advanced to 2.0s and drove the default-clock guard"
    );
}

// ===========================================================================
// Cross-reference (informational, NOT a duplicate gate)
// `clock-currenttime-advances-monotonically` and `timeof-returns-start-shot-time`
// are GATED-elsewhere in runtime_spec_conformance.rs
// (`spec_clock_monotonic_currenttime_exposed_to_guards`,
// `spec_clock_timeof_snapshot_semantics`). Not re-gated here by design.
//
// Model-callable `TimeOf` / `DurationOf` (occurrence-relative time queries from
// SysML source) remain UNIMPLEMENTED — see that file's
// `spec_clock_timeof_snapshot_semantics`. This file gates the OBSERVABLE
// realized invariants (ordering / continuity / duration over the occurrence
// tracker), which DO have a runtime surface.
// ===========================================================================

// ===========================================================================
// OBL — clock-timeflow-constraint
// "The currentTime of a snapshot of a Clock is equal to the TimeOf the snapshot
// relative to that Clock." `Clocks.kerml:47-57` `timeFlowConstraint`
// (LIBRARY):
//     snapshots->forAll{in s : Clock; TimeOf(s, thisClock) == s.currentTime}
//
// This is DISTINCT from `clock-currenttime-advances-monotonically` (which only
// requires currentTime to increase). timeFlowConstraint pins the snapshot↔
// clock-time *identity*: at the instant a snapshot is taken, the clock's
// currentTime must equal that snapshot's own TimeOf relative to the clock — the
// clock reading and the snapshot time are the same number, not merely both
// increasing.
//
// Runtime surface: a snapshot is a zero-duration occurrence (startShot ==
// endShot), so TimeOf(snapshot) == its start time (`clock::time_of`). When the
// clock has been advanced to that snapshot's instant, `Clock::current_time` ==
// `time_of(snapshot_time, &clock)`. Both surfaces exist; the identity holds.
// ===========================================================================

#[test]
fn occ_clock_timeflow_snapshot_currenttime_equals_timeof() {
    // OBL: clock-timeflow-constraint
    // VERDICT: CONFORMS
    // A snapshot is a zero-duration occurrence: take it at instant t = 3.0.
    let snapshot_time = 3.0;
    let tracker = run_occurrence_snapshot("tick", snapshot_time);
    let snap = &tracker.completed()[0];
    // Snapshot is zero-duration (startShot == endShot) — the precondition for it
    // being a Clock snapshot at a single instant.
    assert!(
        (snap.end_time.unwrap() - snap.start_time).abs() < 1e-12,
        "a Clock snapshot is a zero-duration occurrence (startShot == endShot)"
    );

    // Advance the clock to the snapshot's instant — this is the clock state at
    // the moment the snapshot is taken.
    let mut clock = Clock::new("thisClock");
    clock.advance(snapshot_time);

    // timeFlowConstraint: TimeOf(s, thisClock) == s.currentTime.
    // TimeOf(snapshot) is the snapshot's (start) time relative to the clock.
    let timeof_snapshot = time_of(snap.start_time, &clock);
    assert!(
        (timeof_snapshot - clock.current_time).abs() < 1e-12,
        "timeFlowConstraint: TimeOf(snapshot, clock) == clock.currentTime \
         (Clocks.kerml:47-57)"
    );
    // And the snapshot's own time is that shared value (identity, not just
    // monotonic advance): TimeOf(snapshot) == snapshot.time == clock.currentTime.
    assert!(
        (timeof_snapshot - snapshot_time).abs() < 1e-12,
        "the clock reading and the snapshot time are the same number"
    );
}

// ===========================================================================
// OBL — suboccurrence-endshot-coincidence
// "When a parent occurrence ends, each suboccurrence's endShot must be time-
// coincident with the parent's endShot." `Occurrences.kerml:379-382` (LIBRARY):
//     feature subendshot : Occurrence[0..*] chains self.suboccurrences.endShot {
//         feature superendshot : Occurrence[1] subsets that;
//         subset superendshot subsets self.timeCoincidentOccurrences; }
//
// The runtime `OccurrenceTracker` models occurrences as a FLAT set keyed by
// "subsystem:name" (occurrence.rs:53-56) with NO parent/child containment: there
// is no `suboccurrences` relation, no `endShot` snapshot feature, and no
// `timeCoincidentOccurrences` set. The structural surface this obligation
// constrains does not exist, so the coincidence cannot be observed or enforced.
// Pinned UNIMPLEMENTED (not faked over the flat tracker).
// ===========================================================================

#[test]
#[ignore = "UNIMPL: OccurrenceTracker has no sub-occurrence containment (no suboccurrences/endShot); Occurrences.kerml:379-382 requires child.endShot coincide with parent.endShot — no runtime surface to assert against yet"]
fn occ_suboccurrence_endshot_coincident_with_parent() {
    // OBL: suboccurrence-endshot-coincidence
    // VERDICT: UNIMPLEMENTED — OccurrenceTracker has no sub-occurrence
    // containment (no suboccurrences / endShot / timeCoincidentOccurrences).
    // Pin the absence: the tracker exposes only a flat completed-set with no
    // parent→child relation, so there is no surface on which endShot
    // coincidence between a parent and its suboccurrences could be asserted.
    let mut tracker = OccurrenceTracker::new();
    // A "parent" [0,10] and a would-be "suboccurrence" [2,10]. The tracker
    // stores both as peers — it has no notion that the second is contained in
    // the first, hence no way to require their endShots coincide.
    tracker.begin(OccurrenceKind::EventOccurrence, "sys", "parent", 0.0, HashMap::new());
    tracker.begin(OccurrenceKind::EventOccurrence, "sys", "child", 2.0, HashMap::new());
    tracker.end("sys", "child", 10.0, HashMap::new());
    tracker.end("sys", "parent", 10.0, HashMap::new());

    // The completed set is flat — both occurrences are siblings, with no
    // containment metadata distinguishing parent from suboccurrence.
    assert_eq!(
        tracker.completed().len(),
        2,
        "tracker stores occurrences as a flat peer set — no containment hierarchy"
    );
    // There is NO API to enumerate a parent's suboccurrences or its endShot.
    // The Occurrence struct (occurrence.rs:23-41) carries name/kind/subsystem/
    // start/end/duration/features only — no `suboccurrences`, no `endShot`, no
    // `timeCoincidentOccurrences`. The obligation has no runtime surface.
    // (If sub-occurrence containment is added, promote this to a CONFORMS gate
    // asserting child.endShot == parent.endShot when the parent ends.)
}

// ===========================================================================
// Matrix summary — self-scans this file and prints verdict counts.
// ===========================================================================

#[test]
fn occurrence_matrix_summary() {
    let src = include_str!("occurrence_clock_spec_conformance.rs");
    let verdicts: Vec<&str> = src
        .lines()
        .map(|l| l.trim())
        .filter(|l| l.starts_with("// VERDICT: "))
        .collect();
    let conforms = verdicts
        .iter()
        .filter(|l| l.starts_with("// VERDICT: CONFORMS"))
        .count();
    let diverges = verdicts
        .iter()
        .filter(|l| l.starts_with("// VERDICT: DIVERGES"))
        .count();
    let unimpl = verdicts
        .iter()
        .filter(|l| l.starts_with("// VERDICT: UNIMPLEMENTED"))
        .count();
    println!(
        "OCC occurrence/clock matrix: {} gated obligations — \
         {conforms} CONFORMS, {diverges} DIVERGES, {unimpl} UNIMPLEMENTED",
        verdicts.len()
    );
    // The six UNGATED behavioral obligations from the tracker must all be present.
    assert!(
        verdicts.len() >= 6,
        "expected ≥6 verdict-marked gates for the ungated occurrence/clock obligations"
    );
}
