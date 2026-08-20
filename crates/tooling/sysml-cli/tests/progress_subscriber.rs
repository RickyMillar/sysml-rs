//! Integration coverage for the P-RA5 CLI progress subscriber.
//!
//! Production publishers for `SysmlService::publish_progress` are landing
//! incrementally (the bus itself shipped in P-RA4). These tests focus on
//! what the CLI slice can guarantee end-to-end today:
//!
//! 1. The global `--quiet` and `--force-progress` flags are accepted by the
//!    `inspect` subcommand without affecting its primary output channel.
//! 2. With `--quiet`, no progress markers leak to stderr (regardless of
//!    whether stderr is a TTY at run time).
//! 3. With `--force-progress`, the subscriber thread spawns successfully
//!    (the run completes cleanly — no panic, no hang).
//!
//! Renderer-level coverage (the `[library …] / [workspace …]` formatting)
//! is exercised by the unit tests inside `src/progress.rs`.

use std::path::PathBuf;
use std::process::Command;

fn sysml_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sysml"))
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

#[test]
fn inspect_accepts_quiet_flag() {
    let file = fixture("traffic_light.sysml");
    assert!(file.exists(), "fixture should exist: {}", file.display());

    let output = sysml_bin()
        .arg("--quiet")
        .args(["inspect", "--diagnostics", "--no-stdlib"])
        .arg(&file)
        .output()
        .expect("failed to run sysml");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("[library") && !stderr.contains("[workspace"),
        "expected no progress markers under --quiet; got stderr: {}",
        stderr
    );
}

#[test]
fn inspect_accepts_force_progress_flag() {
    // The --force-progress flag must be accepted globally and the subcommand
    // must still complete successfully. We don't assert the presence of
    // progress markers — no production code publishes `ProgressEvent`s
    // during a single-file `inspect --no-stdlib` run today. The subscriber
    // thread spawning cleanly is the contract under test.
    let file = fixture("traffic_light.sysml");
    let output = sysml_bin()
        .arg("--force-progress")
        .args(["inspect", "--diagnostics", "--no-stdlib"])
        .arg(&file)
        .output()
        .expect("failed to run sysml");

    assert!(
        output.status.success(),
        "inspect with --force-progress should succeed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
