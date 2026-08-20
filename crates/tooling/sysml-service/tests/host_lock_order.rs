//! Host lock-order deadlock gate (2026-07-17 server wedge).
//!
//! ## The bug this pins
//!
//! `SysmlService.host` is a salsa `AnalysisHost` behind a
//! `std::sync::Mutex`; `Analysis` snapshots are salsa db clones. Any host
//! MUTATION (`load_workspace` → `set_file_content_in_project`, LSP
//! `did_change`) runs under the Mutex and blocks inside salsa's
//! `zalsa_mut` until every outstanding db clone is dropped.
//!
//! A read path that takes an `Analysis` and THEN re-acquires the host
//! Mutex (e.g. via `source_file_for` / `workspace_pfs`) closes a
//! permanent two-thread cycle against a concurrent mutation:
//!
//! ```text
//! reader:  snapshot alive ── waits on host Mutex
//! loader:  holds host Mutex ── waits (in salsa) for snapshot to drop
//! ```
//!
//! Every subsequent request then queues on the Mutex forever — the
//! production symptom was sysml-api accepting TCP connections and never
//! responding (twice on 2026-07-17, gdb thread dump confirmed exactly the
//! two stacks above). The fix: any path needing a snapshot plus host
//! lookups takes both under ONE lock acquisition
//! (`SysmlService::locked_analysis_with`); see the lock-order invariant
//! documented on `host_analysis`.
//!
//! ## What this test does
//!
//! Hammers the formerly-cyclic read commands (file-scope `require_graph`
//! via `sysml.stats`, `model_tree`, `descendants`, `trace_matrix`) from
//! several threads while two loader threads run `load_workspace`
//! (DiskAuthoritative reload — the mutation arm of the production wedge)
//! in a loop. Pre-fix this wedges within a few hundred milliseconds of
//! loop iterations; post-fix it completes. A watchdog `recv_timeout`
//! turns a regression into a clean test failure instead of a hung CI job.
//!
//! Readers run under `Cancelled::catch` — a reload cancelling in-flight
//! snapshot queries is designed behaviour (surfaced as a transient 500 at
//! the REST layer), not a failure of this gate. Two subtleties learned
//! writing this:
//!
//! - Loaders need the catch too: `load_workspace`'s own post-mutation
//!   parse-error collection reads a snapshot, which the OTHER loader's
//!   mutation can cancel.
//! - Salsa cancellation unwinds via `resume_unwind` (NO panic message),
//!   and when a query participant dies, threads blocked on the same memo
//!   get the panic propagated with salsa's own payload — which is not
//!   `Cancelled` and so escapes `Cancelled::catch`. The readers' outer
//!   `catch_unwind` + unconditional `send` keeps such deaths from
//!   masquerading as a deadlock; the eprintln reports what actually died.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use sysml_service::SysmlService;

const HAMMER_DURATION: Duration = Duration::from_secs(4);
/// Generous bound: pre-fix the wedge is permanent, so any timeout works;
/// keep it far above worst-case healthy completion under CI load.
const WATCHDOG: Duration = Duration::from_secs(120);

fn write_fixture(dir: &std::path::Path) {
    std::fs::write(
        dir.join("a.sysml"),
        "package A { part def Widget { attribute mass : Real = 1.0; } }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("b.sysml"),
        "package B { import A::*; part w : Widget; }\n",
    )
    .unwrap();
}

#[test]
fn concurrent_reads_and_reloads_do_not_deadlock() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_fixture(tmp.path());

    let service = Arc::new(SysmlService::empty());
    let loaded = service.load_workspace(tmp.path()).expect("initial load");
    let file_uri = loaded
        .loaded_uris
        .iter()
        .find(|u| u.ends_with(".sysml"))
        .expect("a file uri")
        .clone();

    let stop = Arc::new(AtomicBool::new(false));
    let (done_tx, done_rx) = mpsc::channel::<&'static str>();
    let mut expected = 0;

    // Readers: the four formerly-cyclic snapshot-then-relock paths, on
    // FILE scope (the arm that used to take `host_analysis()` and then
    // call `source_file_for` / `workspace_pfs` with the snapshot alive).
    for i in 0..4 {
        let service = Arc::clone(&service);
        let stop = Arc::clone(&stop);
        let tx = done_tx.clone();
        let uri = file_uri.clone();
        expected += 1;
        std::thread::spawn(move || {
            let body = std::panic::catch_unwind(AssertUnwindSafe(|| {
            while !stop.load(Ordering::Relaxed) {
                // Cancellation from a concurrent reload is expected; any
                // other panic must surface, not be swallowed.
                let r = sysml_ide_db::Cancelled::catch(AssertUnwindSafe(|| {
                    match i % 4 {
                        0 => {
                            let _ = service.require_graph(&uri);
                        }
                        1 => {
                            let _ = service.model_tree(&uri, Some(3), None);
                        }
                        2 => {
                            // Any id works — the hammer targets the lock
                            // choreography, not the lookup result.
                            let _ = service.descendants(
                                &uri,
                                &sysml_id::ElementId::from_string("lock-order-probe"),
                            );
                        }
                        _ => {
                            let _ = service.trace_matrix(
                                &uri,
                                &sysml_core::ElementKind::PartUsage,
                                &sysml_core::RelationshipKind::Satisfy,
                                &sysml_core::ElementKind::RequirementUsage,
                            );
                        }
                    }
                }));
                let _ = r; // Ok(_) or Err(Cancelled) — both fine.
            }
            }));
            if let Err(p) = body {
                let kind = if p.downcast_ref::<sysml_ide_db::Cancelled>().is_some() {
                    "Cancelled (escaped catch?!)"
                } else if let Some(s) = p.downcast_ref::<&str>() {
                    s
                } else if let Some(s) = p.downcast_ref::<String>() {
                    s.as_str()
                } else {
                    "unknown payload"
                };
                eprintln!("reader {i} DIED: {kind}");
            }
            let _ = tx.send("reader");
        });
    }

    // Loaders: the mutation arm — DiskAuthoritative reload in a loop.
    // A loader's own post-mutation snapshot reads (parse-error collection)
    // can be cancelled by the OTHER loader's mutation — tolerated, same as
    // any snapshot read.
    for _ in 0..2 {
        let service = Arc::clone(&service);
        let stop = Arc::clone(&stop);
        let tx = done_tx.clone();
        let root = tmp.path().to_path_buf();
        expected += 1;
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let r = sysml_ide_db::Cancelled::catch(AssertUnwindSafe(|| {
                    service.load_workspace(&root).expect("reload");
                }));
                let _ = r;
            }
            let _ = tx.send("loader");
        });
    }
    drop(done_tx);

    let start = Instant::now();
    std::thread::sleep(HAMMER_DURATION);
    stop.store(true, Ordering::Relaxed);

    for _ in 0..expected {
        match done_rx.recv_timeout(WATCHDOG) {
            Ok(_) => {}
            Err(e) => panic!(
                "DEADLOCK: worker did not finish within {WATCHDOG:?} after \
                 {:?} of hammering ({e}). A read path is re-acquiring the \
                 host Mutex while an `Analysis` snapshot is alive — see the \
                 lock-order invariant on `SysmlService::host_analysis`.",
                start.elapsed(),
            ),
        }
    }
}
