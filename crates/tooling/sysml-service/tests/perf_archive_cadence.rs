//! UX closeout arc #7 — archive-cadence perf acceptance gate
//!
//! Not a CI regression gate — wall-clock timing on a shared/loaded box is
//! inherently noisy (the design doc itself measured 10.6 ms/tick "on a
//! loaded box" against a quieter theoretical ~13-19 ms/tick). This is a
//! manually-run perf report, `#[ignore]`d so it never blocks `cargo test`.
//! It measures `RuntimeSession::step()` directly (bypassing the JSON
//! `execute_command` marshaling layer, matching how the design doc's own
//! baseline numbers were produced) on the espresso pump-hybrid workspace, comparing:
//!
//! - **dense** (cadence = 1): the pre-arc-7 contract — archive
//!   (`fork_for_archive`, a full `Orchestrator` deep clone) on every tick.
//! - **coarse** (cadence = [`execution::DEFAULT_ARCHIVE_CADENCE_TICKS`]):
//!   the new default — archive only on the cadence + forced-event ticks.
//!
//! Run in **release** mode for a meaningful number (debug builds are
//! dominated by unrelated per-call overhead):
//!
//! ```sh
//! cargo test --release -p sysml-service --test perf_archive_cadence -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::time::Instant;

use serde_json::json;
use sysml_id::ElementId;
use sysml_project::discovery::OpenTarget;
use sysml_service::{execute_command, execution, SysmlService};

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

/// Starts an espresso pump-hybrid orchestrator session at dt=0.0001 ms (the same resolution
/// the design doc profiled) and overrides its snapshot retention + archive
/// cadence to the requested values.
fn start_espresso_session(service: &SysmlService, cadence_ticks: u64) -> String {
    let start = execute_command(
        service,
        "sysml.orchestrate.workspace.start",
        json!({ "uri": "__workspace__", "dt_ms": 0.0001, "max_time_ms": 500.0 }),
    )
    .expect("orchestrate.workspace.start");
    let arr = start
        .as_array()
        .expect("workspace.start returns [key, snapshot]");
    let key = arr[0].as_str().expect("session key").to_owned();

    service
        .set_session_snapshot_retention(&key, execution::DEFAULT_SNAPSHOT_RETENTION_TICKS)
        .expect("set_session_snapshot_retention should find the session");
    service
        .set_session_archive_cadence(&key, cadence_ticks)
        .expect("set_session_archive_cadence should find the session");
    key
}

/// Warms up `warmup` steps (untimed — matches the design doc's profiling
/// harness, which warms 200 ticks before timing), then times `n` further
/// `RuntimeSession::step()` calls. Returns mean ms/tick.
fn time_steps(service: &SysmlService, key: &str, warmup: usize, n: usize) -> f64 {
    let session_key = ElementId::from_string(key);
    {
        let mut entry = service
            .sessions()
            .get_mut(&session_key)
            .expect("session exists");
        for _ in 0..warmup {
            let _ = entry.step();
        }
    }
    let t0 = Instant::now();
    {
        let mut entry = service
            .sessions()
            .get_mut(&session_key)
            .expect("session exists");
        for _ in 0..n {
            let _ = std::hint::black_box(entry.step());
        }
    }
    let elapsed = t0.elapsed();
    elapsed.as_secs_f64() * 1000.0 / n as f64
}

#[test]
#[ignore = "manual perf report — run with --release --ignored --nocapture"]
fn perf_report_archive_cadence_espresso() {
    const WARMUP: usize = 200;
    const N: usize = 3000;

    // OLD contract: cadence=1 means "archive every tick" — exactly what
    // `RuntimeSession::step()` did unconditionally before this arc.
    let dense = SysmlService::empty();
    dense
        .open_context(OpenTarget::Folder(espresso_pump_root()))
        .expect("open espresso pump-hybrid workspace (dense)");
    let dense_key = start_espresso_session(&dense, 1);
    let dense_ms = time_steps(&dense, &dense_key, WARMUP, N);

    // NEW default cadence.
    let coarse = SysmlService::empty();
    coarse
        .open_context(OpenTarget::Folder(espresso_pump_root()))
        .expect("open espresso pump-hybrid workspace (coarse)");
    let coarse_key = start_espresso_session(&coarse, execution::DEFAULT_ARCHIVE_CADENCE_TICKS);
    let coarse_ms = time_steps(&coarse, &coarse_key, WARMUP, N);

    eprintln!("\nUX closeout arc #7 perf report (espresso pump-hybrid, dt=0.0001ms, N={N}):");
    eprintln!("  dense  archive (cadence=1):   {dense_ms:.4} ms/tick");
    eprintln!(
        "  coarse archive (cadence={}): {coarse_ms:.4} ms/tick",
        execution::DEFAULT_ARCHIVE_CADENCE_TICKS
    );
    eprintln!(
        "  speedup: {:.1}x   ({:.4} ms/tick recovered)",
        dense_ms / coarse_ms.max(f64::EPSILON),
        dense_ms - coarse_ms
    );

    assert!(
        coarse_ms < dense_ms,
        "coarse archive cadence must be cheaper per tick than dense (every-tick) archiving \
         (dense={dense_ms:.4}ms coarse={coarse_ms:.4}ms)"
    );
}
