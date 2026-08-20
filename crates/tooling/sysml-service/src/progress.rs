//! Progress broadcast bus (Phase P-RA4).
//!
//! Every loading event a user-facing channel might want to render is a
//! [`ProgressEvent`] on a `tokio::sync::broadcast` channel. Transports
//! (LSP, MCP, REST, CLI) subscribe via [`SysmlService::subscribe_progress`]
//! and translate events to their native progress idiom — `$/progress` +
//! `window/logMessage` on LSP, SSE on REST, stderr lines on CLI.
//!
//! The bus is non-blocking: if no subscriber is attached the
//! [`SysmlService::publish_progress`] call is a no-op. Lagged subscribers
//! drop messages rather than blocking the publisher (capacity 256).
//!

use serde::{Deserialize, Serialize};

/// A single lifecycle event emitted by the service. Cheap to clone:
/// every variant is either `Copy` or holds an `Arc<str>` / `&'static str`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProgressEvent {
    /// Library load phase transition. `done` and `total` are advisory —
    /// pure-transition events (e.g. `Loading` → `Failed`) use `0/0`.
    LibraryLoad {
        phase: LibraryPhase,
        done: u32,
        total: u32,
        /// Free-form context: file count, elapsed-ms, error string.
        /// Empty when the transition itself is all the information a
        /// renderer needs.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        detail: String,
    },
    /// Workspace indexing phase tick. `phase` is the numeric phase id
    /// (1=discovery, 2=load, 3=PFS-build, 4=diagnostics) matching the
    /// LSP `Workspace indexing: ...` log message family.
    WorkspaceIndex {
        phase: u8,
        done: u32,
        total: u32,
        /// Free-form context (file count, elapsed ms, etc.). Empty by
        /// default.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        detail: String,
    },
    /// Dependency fetch progress (registry / git / kpar).
    DependencyFetch {
        name: String,
        done: u32,
        total: u32,
    },
    /// Caller requested clients refresh — e.g. after the library
    /// finishes loading.
    Refresh { reason: &'static str },
    /// Generic "the workspace is now usable" signal.
    Ready,
}

/// Phase tag for [`ProgressEvent::LibraryLoad`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LibraryPhase {
    /// `enable_stdlib()` has been called; load is in flight.
    Loading,
    /// Library finished loading successfully. `done` is the element
    /// count.
    Loaded,
    /// Library load reported a fatal error. `detail` carries the
    /// user-visible cause.
    Failed,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::readiness::LibraryReadiness;
    use crate::SysmlService;

    /// Subscribers attached BEFORE the publish call receive the event.
    /// Publishing a `LibraryLoad` event also updates the service-tracked
    /// `library_lifecycle` override so `readiness_for` reports the new
    /// state.
    #[tokio::test]
    async fn library_load_publish_reaches_subscriber_and_updates_readiness() {
        let service = SysmlService::empty();
        let mut rx = service.subscribe_progress();

        service.publish_progress(ProgressEvent::LibraryLoad {
            phase: LibraryPhase::Loading,
            done: 0,
            total: 0,
            detail: String::new(),
        });

        // Subscriber observes the event.
        let received = rx.recv().await.expect("subscriber receives event");
        assert!(matches!(
            received,
            ProgressEvent::LibraryLoad {
                phase: LibraryPhase::Loading,
                ..
            }
        ));

        // Readiness now reports Loading (overlay, not host-derived).
        let r = service.readiness_for("nonexistent.sysml");
        assert_eq!(r.library, LibraryReadiness::Loading);

        // Publishing Failed updates the override.
        service.publish_progress(ProgressEvent::LibraryLoad {
            phase: LibraryPhase::Failed,
            done: 0,
            total: 0,
            detail: "boom".to_owned(),
        });
        let r = service.readiness_for("nonexistent.sysml");
        match r.library {
            LibraryReadiness::Failed(msg) => assert_eq!(&*msg, "boom"),
            other => panic!("expected Failed, got {:?}", other),
        }
    }

    /// `publish_progress` with no subscriber is a no-op (no panic, no
    /// blocking).
    #[tokio::test]
    async fn publish_without_subscriber_is_noop() {
        let service = SysmlService::empty();
        // No subscribe call.
        service.publish_progress(ProgressEvent::Ready);
        service.publish_progress(ProgressEvent::Refresh { reason: "test" });
        // Adding a subscriber after the publish should not see the
        // old events (broadcast channel does not replay).
        let mut rx = service.subscribe_progress();
        // Push a fresh event and assert that's what arrives first.
        service.publish_progress(ProgressEvent::Ready);
        let r = rx.recv().await.expect("subscriber receives event");
        assert!(matches!(r, ProgressEvent::Ready));
        // Subsequent `try_recv` is empty.
        match rx.try_recv() {
            Err(_) => {} // expected: Empty
            Ok(e) => panic!("unexpected pending event: {:?}", e),
        }
    }

    /// `reset_library_lifecycle` clears any sticky `Failed`/`Loading`
    /// override so `readiness_for` falls back to host-derived state.
    #[tokio::test]
    async fn reset_clears_lifecycle_override() {
        let service = SysmlService::empty();
        service.publish_progress(ProgressEvent::LibraryLoad {
            phase: LibraryPhase::Failed,
            done: 0,
            total: 0,
            detail: "stuck".to_owned(),
        });
        assert!(matches!(
            service.readiness_for("x.sysml").library,
            LibraryReadiness::Failed(_)
        ));
        service.reset_library_lifecycle();
        assert_eq!(
            service.readiness_for("x.sysml").library,
            LibraryReadiness::Unloaded
        );
    }

    /// Lagged subscribers drop messages rather than block the
    /// publisher. We verify by overflowing the channel (capacity 256)
    /// and confirming the publisher never blocks.
    #[tokio::test]
    async fn publisher_never_blocks_when_subscriber_lags() {
        let service = SysmlService::empty();
        let mut rx = service.subscribe_progress();
        for _ in 0..512 {
            service.publish_progress(ProgressEvent::Ready);
        }
        // Drain whatever arrives; lagged subscribers see `RecvError::Lagged`
        // before the live events resume.
        let mut saw_lagged = false;
        for _ in 0..512 {
            match rx.try_recv() {
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                    saw_lagged = true;
                }
                Err(_) => break,
            }
        }
        // Sanity: with capacity 256 and 512 sends, lag is expected.
        assert!(saw_lagged, "expected at least one lag notification");
    }
}
