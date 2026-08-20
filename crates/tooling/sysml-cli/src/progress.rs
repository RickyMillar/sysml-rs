//! Progress rendering for the CLI (P-RA5 CLI slice).
//!
//! Subscribes to `SysmlService::subscribe_progress()` and renders each
//! [`ProgressEvent`] as a single-line update on stderr. Output is suppressed
//! when stderr is not a TTY or `--quiet` is set, so piped stdout (e.g. JSON
//! output from `sysml inspect --json`) is never corrupted.
//!
//! The renderer runs on a dedicated std thread that calls
//! `broadcast::Receiver::blocking_recv` — no tokio runtime required.
//!
use std::io::{self, IsTerminal, Write};
use std::thread::{self, JoinHandle};

use sysml_service::progress::{LibraryPhase, ProgressEvent};
use sysml_service::SysmlService;
use tokio::sync::broadcast::error::RecvError;

/// Decide whether to render progress for this invocation.
///
/// Order of precedence:
/// 1. `--quiet` (or `SYSML_QUIET=1`) → never render.
/// 2. `--force-progress` (or `SYSML_FORCE_PROGRESS=1`) → always render
///    (used by integration tests that can't fake a TTY).
/// 3. Otherwise, render only when stderr is a terminal.
pub fn should_render(quiet: bool, force: bool) -> bool {
    if quiet {
        return false;
    }
    if force {
        return true;
    }
    io::stderr().is_terminal()
}

/// Format a single [`ProgressEvent`] into the one-line stderr representation.
/// Pure function — no I/O, easy to unit-test.
pub fn format_event(event: &ProgressEvent) -> String {
    match event {
        ProgressEvent::LibraryLoad {
            phase,
            done,
            total,
            detail,
        } => {
            let phase_label = match phase {
                LibraryPhase::Loading => "loading",
                LibraryPhase::Loaded => "loaded",
                LibraryPhase::Failed => "failed",
            };
            let counts = format_counts(*done, *total);
            let suffix = format_detail(detail);
            format!("[library {phase_label}{counts}{suffix}]")
        }
        ProgressEvent::WorkspaceIndex {
            phase,
            done,
            total,
            detail,
        } => {
            let phase_label = match phase {
                1 => "discovery",
                2 => "load",
                3 => "pfs",
                4 => "diagnostics",
                _ => "indexing",
            };
            let counts = format_counts(*done, *total);
            let suffix = format_detail(detail);
            format!("[workspace {phase_label}{counts}{suffix}]")
        }
        ProgressEvent::DependencyFetch { name, done, total } => {
            let counts = format_counts(*done, *total);
            format!("[deps {name}{counts}]")
        }
        ProgressEvent::Refresh { reason } => format!("[refresh {reason}]"),
        ProgressEvent::Ready => "[ready]".to_owned(),
    }
}

fn format_counts(done: u32, total: u32) -> String {
    if total == 0 && done == 0 {
        String::new()
    } else if total == 0 {
        format!(" {done}")
    } else {
        format!(" {done}/{total}")
    }
}

fn format_detail(detail: &str) -> String {
    if detail.is_empty() {
        String::new()
    } else {
        format!(" {detail}")
    }
}

/// Spawn a background thread that consumes the service's progress bus
/// and renders each event to stderr until the publisher is dropped.
///
/// The returned [`JoinHandle`] is detached at end-of-process; callers may
/// drop it. The thread exits cleanly when the broadcast sender is dropped
/// (i.e. when the `SysmlService` is dropped).
pub fn spawn_subscriber(service: &SysmlService) -> JoinHandle<()> {
    let mut rx = service.subscribe_progress();
    thread::Builder::new()
        .name("sysml-cli-progress".to_owned())
        .spawn(move || loop {
            match rx.blocking_recv() {
                Ok(event) => {
                    let line = format_event(&event);
                    let mut stderr = io::stderr().lock();
                    // Ignore write errors — stderr being closed is non-fatal
                    // for the rest of the CLI run.
                    let _ = writeln!(stderr, "{line}");
                    let _ = stderr.flush();
                }
                Err(RecvError::Lagged(_)) => {
                    // Skipped messages; keep looping.
                    continue;
                }
                Err(RecvError::Closed) => break,
            }
        })
        .expect("failed to spawn sysml-cli-progress thread")
}

/// Convenience entry point: if rendering is enabled, spawn the subscriber
/// and return its handle; otherwise return `None`.
pub fn maybe_spawn(service: &SysmlService, quiet: bool, force: bool) -> Option<JoinHandle<()>> {
    if should_render(quiet, force) {
        Some(spawn_subscriber(service))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_library_loading() {
        let e = ProgressEvent::LibraryLoad {
            phase: LibraryPhase::Loading,
            done: 0,
            total: 0,
            detail: String::new(),
        };
        assert_eq!(format_event(&e), "[library loading]");
    }

    #[test]
    fn format_library_loaded_with_count() {
        let e = ProgressEvent::LibraryLoad {
            phase: LibraryPhase::Loaded,
            done: 109,
            total: 109,
            detail: String::new(),
        };
        assert_eq!(format_event(&e), "[library loaded 109/109]");
    }

    #[test]
    fn format_library_failed_with_detail() {
        let e = ProgressEvent::LibraryLoad {
            phase: LibraryPhase::Failed,
            done: 0,
            total: 0,
            detail: "missing SYSML_LIBRARY_PATH".to_owned(),
        };
        assert_eq!(
            format_event(&e),
            "[library failed missing SYSML_LIBRARY_PATH]"
        );
    }

    #[test]
    fn format_workspace_index_phases() {
        let e = ProgressEvent::WorkspaceIndex {
            phase: 2,
            done: 45,
            total: 109,
            detail: String::new(),
        };
        assert_eq!(format_event(&e), "[workspace load 45/109]");
    }

    #[test]
    fn format_dependency_fetch() {
        let e = ProgressEvent::DependencyFetch {
            name: "kernel".to_owned(),
            done: 3,
            total: 7,
        };
        assert_eq!(format_event(&e), "[deps kernel 3/7]");
    }

    #[test]
    fn format_refresh_and_ready() {
        assert_eq!(
            format_event(&ProgressEvent::Refresh { reason: "library" }),
            "[refresh library]"
        );
        assert_eq!(format_event(&ProgressEvent::Ready), "[ready]");
    }

    #[test]
    fn should_render_obeys_quiet() {
        assert!(!should_render(true, false));
        assert!(!should_render(true, true));
    }

    #[test]
    fn should_render_honours_force_when_not_quiet() {
        // Even when stderr is not a TTY (the typical `cargo test` env),
        // force=true must enable rendering.
        assert!(should_render(false, true));
    }
}
