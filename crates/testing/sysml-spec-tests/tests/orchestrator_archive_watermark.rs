//! RW-4 — `orchestrator_archive` memory-watermark gate.
//!
//! Gates `S3.T10` per ADR-011 §5 (pin-by-default `orchestrator_archive`
//! policy / Q-RT-4) and §"Risks" RW-4.
//!
//! ## Why this test exists
//!
//! Per ADR-011 §5, `RuntimeSession::orchestrator_archive` is a ring
//! buffer of `Orchestrator` clones used to power `fork_with_overrides
//! (at_tick = Some(t))`. Each clone retains the *original* `Snapshot`
//! it was built against — pinning by default is the conservative
//! answer because workspace edits during a long-running session must
//! not silently invalidate fork points.
//!
//! The implication ADR-011 §"Risks" RW-4 flags: pinning a snapshot
//! per archived clone blocks salsa from collecting older elaborated
//! graph revisions while the archive still references them. The
//! mitigation is the 256-clone ring-buffer cap
//! (`DEFAULT_SNAPSHOT_RETENTION_TICKS`): each new step evicts the
//! oldest archived clone, freeing its referenced graph revision once
//! no other archived clone holds a reference. Memory growth from this
//! mechanism is therefore O(retention × snapshot_size), independent
//! of edit churn.
//!
//! ## What this test asserts
//!
//! Two invariants on the archive's interaction with workspace edits:
//!
//! 1. **Bounded ring-buffer.** After N >> retention steps, the
//!    archive holds exactly `snapshot_retention_ticks` entries — not
//!    one more. The eviction-on-step path is the memory ceiling.
//!
//! 2. **Pin survives edits.** A `load_source` edit between steps
//!    bumps the salsa revision and produces a fresh elaborated
//!    workspace graph. Archived clones built against the *previous*
//!    revision must remain self-contained and forkable; the past-tick
//!    fork must succeed against an elaborated-graph revision that is
//!    no longer the workspace's current view.
//!
//! ## Why this test stays after T10
//!
//! Going forward, any change to `RuntimeSession::orchestrator_archive`,
//! to `fork_child_at`, or to the snapshot retention bound must keep
//! these two invariants. The test becomes the permanent RW-4 gate
//! protecting the orchestrator_archive memory contract.
//!
//! The test uses a small retention bound (10–20 ticks) rather than
//! the production default of 256 so it runs in <5 s; the
//! eviction-on-step code path is the same regardless of N.
//!
//! ## Cadence (2026-07-17 re-bless)
//!
//! 2026-07-13) changed the recording gate: a tick is archived only when
//! it lands on `archive_cadence_ticks` (default 100) OR is
//! event-significant (breakpoint / state transition / bool flip). The
//! two RW-4 invariants below therefore run at `set_archive_cadence(1)`
//! — per-tick archiving, the same push/evict code path — because they
//! assert on exact per-tick watermarks. `rw4_cadence_gates_archive_pushes`
//! pins the cadence gate itself: below-cadence uneventful ticks must NOT
//! be archived. The fixture is the generic espresso-production-cell
//! workspace; the chosen state def (`GrantedLogic`) settles into a
//! wait-for-message state and fires no further transitions under plain
//! stepping (no port message is injected), so the event-significant
//! OR-arm contributes no extra pushes over the stepped window and the
//! count proves the cadence gate alone decides what is archived.

use std::path::PathBuf;

use sysml_service::SysmlService;

/// A state def in the espresso-production-cell workspace that, once it
/// has taken its initial `entry -> waiting` transition, blocks waiting
/// for a port message that plain stepping never delivers — so it fires
/// no further transitions over the stepped window. That "uneventful"
/// property is what lets the cadence gate (invariant 3) assert exact
/// per-tick archive watermarks.
const SM_NAME: &str = "GrantedLogic";

/// Locate the espresso-production-cell fixture relative to this crate's
/// manifest (repo-root `examples/espresso-production-cell`). The same
/// three-parent hop the sibling espresso service test uses
/// (`crates/tooling/sysml-service/tests/espresso_cell_service.rs`).
fn espresso_cell_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("..")
        .join("examples")
        .join("espresso-production-cell")
}

fn load_service() -> SysmlService {
    let service = SysmlService::empty();
    service
        .load_workspace(&espresso_cell_dir())
        .expect("load espresso-production-cell");
    service
}

/// Invariant 1 — the archive ring buffer stops growing at the
/// configured retention bound. After 5× retention steps, exactly
/// `retention` entries remain.
#[test]
fn rw4_archive_bounded_under_extended_stepping() {
    let service = load_service();
    let (key, _initial) = service
        .simulate_start("__workspace__", SM_NAME)
        .expect("simulate_start");
    let key_str = key.to_string();

    let retention: usize = 10;
    {
        let mut entry = service
            .sessions()
            .get_mut(&key)
            .expect("session present after simulate_start");
        entry.set_snapshot_retention(retention);
        // Per-tick archiving: this invariant asserts exact per-tick
        // watermarks (count, earliest/latest), which the default
        // cadence (100) deliberately does not provide. See module doc.
        entry.set_archive_cadence(1);
    }

    // `simulate_start` itself fires one initial step (see
    // `SysmlService::simulate_start` body), so the post-start latest
    // tick is 1, not 0. Account for that when computing the expected
    // earliest/latest below.
    let steps = retention * 5;
    for _ in 0..steps {
        service
            .simulate_step(&key_str, None)
            .expect("simulate_step");
    }

    let session = service
        .sessions()
        .get(&key)
        .expect("session present after stepping");
    assert_eq!(
        session.archived_snapshot_count(),
        retention,
        "archive ring buffer must cap at snapshot_retention_ticks ({retention}) — \
         RW-4 memory bound depends on eviction-on-step"
    );
    let latest = session.latest_tick();
    let earliest = session
        .earliest_archived_tick()
        .expect("archive non-empty after stepping");
    let expected_latest = (steps + 1) as u64;
    assert_eq!(
        latest, expected_latest,
        "latest tick = {steps} explicit steps + 1 implicit step from simulate_start"
    );
    assert_eq!(
        earliest,
        expected_latest - retention as u64 + 1,
        "earliest archived tick should be (latest - retention + 1) after eviction"
    );
}

/// Invariant 3 — the cadence gate itself (archive-cadence arc,
/// 2026-07-13): with `archive_cadence_ticks = N > 1`, only ticks on
/// the cadence land in the archive; uneventful off-cadence ticks are
/// deliberately NOT archived. This is the contract the two per-tick
/// invariants above opt out of via `set_archive_cadence(1)` — pin it
/// so a regression in the gate (e.g. archiving every tick again, or
/// never archiving) is caught here rather than by memory blow-up or a
/// silently empty archive in the fork/rewind UX.
///
/// `GrantedLogic` fires no state transitions / bool flips over this
/// window under plain stepping: after its initial `entry -> waiting`
/// transition it blocks on a port message that is never injected, so
/// the event-significant OR-arm contributes no extra pushes here and
/// the cadence gate alone decides what is archived.
#[test]
fn rw4_cadence_gates_archive_pushes() {
    let service = load_service();
    let (key, _initial) = service
        .simulate_start("__workspace__", SM_NAME)
        .expect("simulate_start");
    let key_str = key.to_string();

    let cadence: u64 = 5;
    {
        let mut entry = service
            .sessions()
            .get_mut(&key)
            .expect("session present after simulate_start");
        entry.set_snapshot_retention(10);
        entry.set_archive_cadence(cadence);
    }

    // simulate_start fired tick 1; step to tick 13.
    for _ in 0..12 {
        service
            .simulate_step(&key_str, None)
            .expect("simulate_step");
    }

    let session = service.sessions().get(&key).expect("session");
    assert_eq!(session.latest_tick(), 13, "1 implicit + 12 explicit steps");
    assert_eq!(
        session.archived_snapshot_count(),
        2,
        "ticks 5 and 10 are the only cadence-eligible ticks in 1..=13"
    );
    for tick in [5_u64, 10] {
        assert!(
            session.has_archived_tick(tick),
            "cadence tick {tick} must be archived"
        );
    }
    for tick in [1_u64, 4, 6, 9, 11, 13] {
        assert!(
            !session.has_archived_tick(tick),
            "off-cadence uneventful tick {tick} must NOT be archived"
        );
    }
    drop(session);
    let _ = service.sessions_stop(&key_str);
}

/// Invariant 2 — workspace edits between steps bump the salsa
/// revision and produce a fresh elaborated graph, but the archived
/// orchestrator clones remain self-contained: forking at a retained
/// past tick succeeds against the *pre-edit* elaborated graph the
/// clone was built against.
#[test]
fn rw4_pin_survives_workspace_edits() {
    let service = load_service();
    let (key, _initial) = service
        .simulate_start("__workspace__", SM_NAME)
        .expect("simulate_start");
    let key_str = key.to_string();

    let retention: usize = 20;
    {
        let mut entry = service
            .sessions()
            .get_mut(&key)
            .expect("session present after simulate_start");
        entry.set_snapshot_retention(retention);
        // Per-tick archiving — the pin target below is `latest - 2`,
        // an arbitrary tick the cadence gate would otherwise skip.
        entry.set_archive_cadence(1);
    }

    // Phase 1 — fill the archive to retention.
    for _ in 0..retention {
        service
            .simulate_step(&key_str, None)
            .expect("simulate_step phase 1");
    }

    let pre_edit_target_tick = {
        let session = service.sessions().get(&key).expect("session");
        let latest = session.latest_tick();
        // Pick a tick near the END of the retention window. Phase 2
        // adds ~10 more steps; with retention=20 that's well inside
        // the post-churn window [latest - retention + 1 .. new_latest].
        latest - 2
    };

    // Phase 2 — simulate LSP keystroke churn. `load_source` calls bump
    // the salsa revision; the next read of the workspace graph
    // produces a fresh elaborated graph. Re-load the same source byte
    // payload each iteration so the parser keeps succeeding; the
    // salsa input change is enough to trigger revision invalidation.
    let edit_target_uri = service
        .loaded_uris()
        .into_iter()
        .find(|u| u.ends_with(".sysml"))
        .expect("at least one .sysml URI loaded from espresso-production-cell");
    let path = edit_target_uri
        .strip_prefix("file://")
        .unwrap_or(&edit_target_uri);
    let original = std::fs::read_to_string(path).expect("read original source");

    for i in 0..5 {
        let edited = format!("{original}\n// rw4 edit {i}\n");
        service
            .load_source(&edit_target_uri, &edited)
            .expect("load_source edit");
        // Step a couple of times — confirms post-edit stepping still
        // works on the existing archived/active orchestrator (which
        // still holds the pre-edit Snapshot). Keep step count low so
        // we don't evict our target tick.
        for _ in 0..2 {
            service
                .simulate_step(&key_str, None)
                .expect("simulate_step post-edit");
        }
    }

    // Restore original source so we don't pollute the working tree.
    service
        .load_source(&edit_target_uri, &original)
        .expect("restore original source");

    // Phase 3 — confirm a fork at the retained past tick still
    // succeeds. The archived clone holds its pre-edit Snapshot; the
    // workspace's current elaborated graph is a later revision; the
    // fork must consult the archive, not the live graph.
    let session = service.sessions().get(&key).expect("session");
    assert!(
        session.has_archived_tick(pre_edit_target_tick),
        "target tick {pre_edit_target_tick} must still be in archive \
         (retention={retention}, latest={})",
        session.latest_tick()
    );
    drop(session);

    let summary = service
        .sessions_fork_with_overrides(&key_str, &[], Some(pre_edit_target_tick))
        .expect(
            "fork at past tick must succeed after workspace edits — \
             archived clones must remain self-contained (pin invariant)",
        );
    assert_eq!(
        summary.fork_point_tick,
        Some(pre_edit_target_tick),
        "child session should record the fork point"
    );

    // Stop the child session immediately — we only care that the fork
    // succeeded. Keeps the session table clean for any sibling tests
    // that share this process.
    let _ = service.sessions_stop(&summary.id);
}
