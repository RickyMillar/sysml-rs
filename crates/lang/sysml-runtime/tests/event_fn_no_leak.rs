//! Purpose-built no-leak fixture for task #10 (steward ruling; the full
//! narrative and the oscillator-shaped regression coverage lived in a sibling
//! slot-identity test since removed with the legacy oscillator fixture).
//!
//! That sibling test inferred the fix from a full oscillator-fixture run's slot/name-map
//! agreement — a signal that, empirically, does NOT go red on a bare revert
//! of `EvalContext::demote_to_read_only` in this particular model (the
//! duty-shadow-refresh half of the fix happens to make the leaked write
//! land the same value the coherent path would have written anyway, once
//! `duty` has settled). It is not proof that the demotion is load-bearing.
//!
//! This fixture isolates the leak directly against the production
//! `ZeroCrossingDetector::check` API (`ode_events.rs`) with a hand-built
//! `EventFn` that mirrors `wire_when_crossings_for_pair`'s exact pattern
//! (`compiler.rs`): clone the live context, demote it, then `set()` a
//! probe slot to a value derived from the (necessarily-interpolated,
//! bisection-probe) `y` it's given. Bisection guarantees the probe fires at
//! several different, non-final `y` values before `check()` returns — if
//! any of those speculative writes ever reached the master `SlotStore`, the
//! probe slot would end up holding whatever the LAST probe computed, not
//! the pre-call value. Asserting the master slot is byte-identical
//! before/after `check()` proves the absence of the leak directly, with no
//! dependence on whether a full model run happens to produce a value large
//! enough to notice.
use std::sync::{Arc, RwLock};

use sysml_core::Value;
use sysml_runtime::expressions::EvalContext;
use sysml_runtime::ode_events::{CrossingDirection, EventFn, ZeroCrossingDetector};
use sysml_runtime::slots::{RuntimeId, SlotMeta, SlotStore, Variability, WriterId};

/// Build a one-slot `EvalContext` with a live write handle (`slots`, not
/// `slot_reader`) — the same shape the orchestrator hands `event_fn` via
/// `detector.check(..., &self.context)` (`orchestrator.rs`'s
/// `handle_crossing_detector`).
fn context_with_probe_slot(name: &str, initial: f64) -> (EvalContext, sysml_runtime::slots::SlotId) {
    let mut store = SlotStore::new();
    let runtime_id = RuntimeId::top_level(sysml_core::ElementId::new_v4());
    let meta = SlotMeta::new(
        runtime_id,
        Variability::Continuous,
        WriterId::Orchestrator,
        name,
        name,
    );
    let slot_id = store.intern(meta, Value::Float(initial));

    let mut ctx = EvalContext::new();
    ctx.slots = Some(Arc::new(RwLock::new(store)));
    (ctx, slot_id)
}

/// An `event_fn` shaped exactly like `wire_when_crossings_for_pair`'s
/// (`compiler.rs` ~7341+): clone, demote, recompute a "signal" from the
/// probe `y`, write it into the clone. `g(t, y, ctx) = y[0]` is the crossing
/// condition (zero at the midpoint of `[-1.0, 1.0]`), so bisection is forced
/// to sample several distinct, non-endpoint `y` values before converging.
fn leak_prone_event_fn(probe_name: &'static str) -> EventFn {
    Arc::new(move |_t, y, ctx| {
        let mut scoped = ctx.alias_live();
        scoped.demote_to_read_only();
        debug_assert!(
            scoped.slots.is_none(),
            "event_fn must never retain a live slot write handle"
        );
        // Mirrors the real closure's signal recompute: derive the "signal"
        // from the (possibly mid-bisection, non-final) interpolated state.
        scoped.set(probe_name.to_owned(), Value::Float(y[0] * 100.0));
        y[0]
    })
}

#[test]
fn zero_crossing_check_never_mutates_the_master_slot_store() {
    let probe_name = "probe";
    let (ctx, slot_id) = context_with_probe_slot(probe_name, 42.0);

    let mut detector = ZeroCrossingDetector::new();
    detector.add_event(
        "crossing",
        CrossingDirection::Either,
        leak_prone_event_fn(probe_name),
    );

    let before = ctx
        .get_slot(slot_id)
        .and_then(|v| v.as_float())
        .expect("probe slot holds a float before check()");
    assert_eq!(before, 42.0, "sanity: probe seeded at 42.0");

    // y crosses zero over [t_start, t_end] — forces `locate_crossing` /
    // `bisect_crossing` to run, invoking `event_fn` at multiple
    // interpolated `y` values strictly between `y_start` and `y_end`
    // (every one of which computes and speculatively "writes" a DIFFERENT
    // probe value than 42.0 into the scoped clone).
    let crossings = detector.check(0.0, 1.0, &[-1.0], &[1.0], &ctx);
    assert_eq!(
        crossings.len(),
        1,
        "test setup regression: bisection must actually run for this fixture \
         to prove anything — the crossing failed to locate"
    );

    let after = ctx
        .get_slot(slot_id)
        .and_then(|v| v.as_float())
        .expect("probe slot holds a float after check()");
    assert_eq!(
        after, before,
        "ZeroCrossingDetector::check mutated the master SlotStore's probe slot \
         (42.0 -> {after}) — the event_fn write-leak (task #10) is back: \
         `EvalContext::demote_to_read_only` is no longer preventing the cloned, \
         speculative bisection context from writing through to production state."
    );
}

/// Companion negative control: with the demotion `event_fn` deliberately
/// left out (i.e. calling `scoped.set()` on a plain `ctx.clone()`, exactly
/// what `wire_when_crossings_for_pair` looked like before the task #10
/// fix), the same fixture DOES observe the leak — proving this test would
/// have caught the original bug and isn't vacuously passing.
#[test]
fn without_demotion_the_leak_reproduces() {
    let probe_name = "probe";
    let (ctx, slot_id) = context_with_probe_slot(probe_name, 42.0);

    let mut detector = ZeroCrossingDetector::new();
    let leaking_event_fn: EventFn = Arc::new(move |_t, y, ctx| {
        // Deliberately NOT demoted — the pre-fix shape.
        let mut scoped = ctx.alias_live();
        scoped.set(probe_name.to_owned(), Value::Float(y[0] * 100.0));
        y[0]
    });
    detector.add_event("crossing", CrossingDirection::Either, leaking_event_fn);

    let crossings = detector.check(0.0, 1.0, &[-1.0], &[1.0], &ctx);
    assert_eq!(crossings.len(), 1, "test setup regression: bisection must run");

    let after = ctx
        .get_slot(slot_id)
        .and_then(|v| v.as_float())
        .expect("probe slot holds a float after check()");
    assert_ne!(
        after, 42.0,
        "negative control failed: expected the undemoted clone to leak a write \
         through to the master SlotStore, but the probe slot is untouched — this \
         fixture's premise (that check() drives event_fn with a live-slots ctx and \
         a bare `ctx.clone()` aliases the store) no longer holds and needs review"
    );
}
