//! PROF1 — bounded per-tick memory-growth regression gate
//!
//! Guards against the exact class of bug that crashed the host this
//! session: an unbounded per-tick accumulator. The `OccurrenceTracker` leak
//! (an unbounded `Vec<Occurrence>` deep-cloned into every archived
//! snapshot) grew resident memory by ~180 KB/tick — 14-25 GB over a long
//! session — and was invisible until it crashed the box; it was only
//! diagnosed by hand-sampling `/proc/<pid>/status` between bulk-steps (see
//! commit 6367c32c, "Fix unbounded occurrence/observational-history memory
//! leak in session archive"). This test makes that check automatic.
//!
//! Measures resident memory directly via `VmRSS` from `/proc/self/status`
//! — the same ground-truth metric used to catch the leak by hand — rather
//! than plumbing in a global allocator (a process-wide change that would
//! risk interfering with every other test in the binary).
//!
//! Two false-positive traps this test is deliberately built to avoid:
//!
//! 1. **The rewind/snapshot archive is a bounded ring, not unbounded, but
//!    it still ramps.** `RuntimeSession` keeps up to
//!    `DEFAULT_SNAPSHOT_RETENTION_TICKS` (256) archived orchestrator
//!    clones. That ring *fills* over the first ~25,600 ticks
//!    (256 slots × ~100-tick archive cadence) even with zero leaks
//!    anywhere else, and each clone is multi-MB — so RSS legitimately
//!    ramps during that window on perfectly healthy code. We disable the
//!    archive for this test via `RuntimeSession::set_snapshot_retention(0)`
//!    so the measurement window has a flat baseline: any *sustained*
//!    growth left over is a real per-tick leak (the leaks we're guarding
//!    against — occurrence tracker, port inbox, succession queue — all
//!    grow independently of the archive).
//! 2. **One-time allocations must be allowed to settle before measuring.**
//!    Workspace compile, first-tick lazy inits, and the time-series ring's
//!    fixed pre-allocation all show up as "growth" on tick 1 but are flat
//!    forever after. We step a warmup window, untimed, before taking the
//!    baseline RSS sample.
//!
//! Run manually (release recommended — see runtime rationale below):
//! ```sh
//! cargo test --release -p sysml-service --test perf_memory_growth_gate \
//!     -- --ignored --nocapture
//! ```

use std::fs;
use std::path::PathBuf;

use serde_json::json;
use sysml_id::ElementId;
use sysml_project::discovery::OpenTarget;
use sysml_service::{execute_command, SysmlService};

fn espresso_pump_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/espresso-pump-hybrid")
}

/// Reads this process's resident set size (`VmRSS`) from
/// `/proc/self/status`, in KB. Linux-only (matches the dev/CI host); no
/// allocator plumbing, no new dependency — RSS is exactly the metric that
/// crashed the box, so it's the right ground truth for this gate.
fn rss_kb() -> u64 {
    let status = fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();
            return digits.parse().expect("parse VmRSS value from /proc/self/status");
        }
    }
    panic!("VmRSS not found in /proc/self/status");
}

/// Ticks stepped untimed before the baseline RSS sample, letting one-time
/// allocations (compile, first-tick lazy inits, time-series pre-alloc)
/// settle. Chosen well past the point those settle on the espresso pump-hybrid model.
const WARMUP_TICKS: usize = 3000;
/// Ticks stepped between the two RSS samples that are actually compared.
const MEASURE_TICKS: usize = 8000;
/// Growth-per-tick ceiling. Generous enough to absorb page-granularity
/// (4 KB) and allocator-retention noise on a bounded run, but two orders
/// of magnitude below the ~180 KB/tick occurrence-tracker leak this test
/// exists to catch — any real per-tick leak trips it long before the host
/// is at risk.
const THRESHOLD_KB_PER_TICK: f64 = 2.0;

/// Perf-lane gate, not a default `cargo test` member: `#[ignore]`d because
/// `WARMUP_TICKS + MEASURE_TICKS` = 11,000 `RuntimeSession::step()` calls
/// plus a fresh espresso pump-hybrid workspace compile is calibrated against the sibling
/// `perf_archive_cadence` test's own release-mode numbers (~0.5 ms/tick for
/// 6,400 total steps there) — comfortably fast in `--release`, but debug
/// builds in this codebase run numerically-heavy ODE/state-machine ticks
/// roughly an order of magnitude slower, which would push this well past
/// the ~20s a default-suite test should cost and make `cargo test` (which
/// builds debug) noticeably slower for everyone. Following the established
/// convention of the sibling perf test in this same crate: keep it
/// runnable on demand, opt-in via `--ignored`, and documented as
/// release-mode.
#[test]
#[ignore = "perf-lane regression gate — run in release: \
            cargo test --release -p sysml-service --test perf_memory_growth_gate -- --ignored --nocapture"]
fn perf_memory_growth_gate_espresso() {
    let service = SysmlService::empty();
    service
        .open_context(OpenTarget::Folder(espresso_pump_root()))
        .expect("open espresso pump-hybrid workspace");

    let start = execute_command(
        &service,
        "sysml.orchestrate.workspace.start",
        json!({ "uri": "__workspace__", "dt_ms": 0.0001, "max_time_ms": 500.0 }),
    )
    .expect("orchestrate.workspace.start");
    let arr = start
        .as_array()
        .expect("workspace.start returns [key, snapshot]");
    let key = arr[0].as_str().expect("session key").to_owned();
    let session_key = ElementId::from_string(&key);

    {
        let mut entry = service
            .sessions()
            .get_mut(&session_key)
            .expect("session exists");

        // Trap 1: disable the rewind archive so its bounded 256-slot
        // ramp-to-fill doesn't masquerade as a leak.
        entry.set_snapshot_retention(0);

        // Trap 2: warm up, untimed, before measuring.
        for _ in 0..WARMUP_TICKS {
            let _ = std::hint::black_box(entry.step());
        }
    }

    let rss_warmup = rss_kb();

    {
        let mut entry = service
            .sessions()
            .get_mut(&session_key)
            .expect("session exists");
        for _ in 0..MEASURE_TICKS {
            let _ = std::hint::black_box(entry.step());
        }
    }

    let rss_after = rss_kb();
    let growth_kb = rss_after as f64 - rss_warmup as f64;
    let kb_per_tick = growth_kb / MEASURE_TICKS as f64;

    eprintln!(
        "\nPROF1 memory-growth gate (espresso pump-hybrid, retention=0, warmup={WARMUP_TICKS}, measure={MEASURE_TICKS}):"
    );
    eprintln!("  rss_warmup: {rss_warmup} KB");
    eprintln!("  rss_after:  {rss_after} KB");
    eprintln!(
        "  growth:     {growth_kb:.1} KB over {MEASURE_TICKS} ticks ({kb_per_tick:.4} KB/tick)"
    );

    assert!(
        kb_per_tick < THRESHOLD_KB_PER_TICK,
        "unbounded per-tick memory growth detected: {kb_per_tick:.4} KB/tick over \
         {MEASURE_TICKS} ticks (threshold {THRESHOLD_KB_PER_TICK} KB/tick, rss_warmup={rss_warmup}KB \
         rss_after={rss_after}KB). This is the occurrence-tracker leak class (~180 KB/tick) — \
         check per-tick accumulators (occurrence tracker, port inbox, succession queue, and any \
         other Vec/VecDeque/HashMap pushed to every tick) for something that grows without a \
         matching pop/clear/truncate."
    );
}
