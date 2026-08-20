//! Periodic session reaper.
//!
//! Pre-S2.T15 each transport binary spawned its own near-identical
//! reaper task (`sysml-api/src/lib.rs::spawn_session_reaper` and
//! `sysml-mcp/src/main.rs`'s inline `tokio::spawn`). Both ticked every
//! 30 s, called `service.sessions_reap()`, and held a `Weak` so the
//! task self-terminated when the service dropped.
//!
//! T15 collapses both into [`spawn_session_reaper`] here. Each binary
//! calls this helper at startup; lifecycle (abort on drop, ignore when
//! no runtime is available) stays at the call site.

use std::sync::Arc;
use std::time::Duration;

use crate::SysmlService;

/// Default cadence for the periodic reaper. 30 s — matches the legacy
/// per-binary reapers this consolidates.
pub const DEFAULT_SESSION_REAPER_INTERVAL: Duration = Duration::from_secs(30);

/// Spawn the periodic session reaper at the default cadence
/// ([`DEFAULT_SESSION_REAPER_INTERVAL`]).
///
/// See [`spawn_session_reaper_with_interval`] for the contract.
pub fn spawn_session_reaper(service: &Arc<SysmlService>) -> Option<tokio::task::AbortHandle> {
    spawn_session_reaper_with_interval(service, DEFAULT_SESSION_REAPER_INTERVAL)
}

/// Spawn the periodic session reaper at a caller-provided cadence.
///
/// Returns `Some(AbortHandle)` when a tokio runtime is available;
/// `None` when called outside a runtime (e.g. a sync test).
///
/// The task captures a `Weak` reference to the service so it
/// self-terminates when the last strong `Arc<SysmlService>` is dropped.
/// Callers that want explicit lifecycle control (e.g. dropping the
/// abort handle in tests) can keep the returned `AbortHandle`; callers
/// that just want the reaper to run for the lifetime of the process
/// can drop it.
pub fn spawn_session_reaper_with_interval(
    service: &Arc<SysmlService>,
    interval: Duration,
) -> Option<tokio::task::AbortHandle> {
    let handle = tokio::runtime::Handle::try_current().ok()?;
    let weak = Arc::downgrade(service);
    let join_handle = handle.spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let Some(service) = weak.upgrade() else { break };
            let _ = service.sessions_reap();
            drop(service);
        }
    });
    Some(join_handle.abort_handle())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn returns_none_outside_a_tokio_runtime() {
        let service = Arc::new(SysmlService::empty());
        assert!(spawn_session_reaper(&service).is_none());
    }

    #[tokio::test]
    async fn spawns_inside_a_tokio_runtime() {
        let service = Arc::new(SysmlService::empty());
        let handle = spawn_session_reaper(&service);
        assert!(handle.is_some());
        // Abort cleanly so the test runtime can shut down.
        if let Some(h) = handle {
            h.abort();
        }
    }

    #[tokio::test]
    async fn task_self_terminates_when_service_drops() {
        // Use a 50 ms cadence so the reaper actually wakes up before
        // the test exits. The contract: dropping the last strong Arc
        // should let the task observe `weak.upgrade() == None` on its
        // next tick and break out of the loop.
        let service = Arc::new(SysmlService::empty());
        let handle = spawn_session_reaper_with_interval(&service, Duration::from_millis(50));
        let handle = handle.expect("runtime is available");
        // Drop the strong Arc — only the Weak inside the task remains.
        drop(service);
        // Give the ticker enough beats to observe the drop and exit.
        // We can't deterministically `await` task completion through
        // the AbortHandle alone, but we can verify the handle is
        // abortable (a task that never woke up is also abortable, so
        // this is a smoke test).
        tokio::time::sleep(Duration::from_millis(150)).await;
        handle.abort();
    }
}
