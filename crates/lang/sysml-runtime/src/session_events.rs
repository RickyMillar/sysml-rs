//! Wire-format frame types for the session-events stream.
//!
//! A streaming client receives:
//! 1. one `Hello` frame at connect with the `base` normalized snapshot,
//! 2. one `Tick` frame per orchestrator tick, carrying a [`DeltaFrame`],
//! 3. an optional `Verdict` frame whenever the verdict rollup changes,
//! 4. a `Completed` frame when the session finishes,
//! 5. an `Error` frame before the server closes the socket on failure.
//!
//! All frames serialize as externally-tagged JSON:
//!
//! ```jsonc
//! { "type": "hello",     "schema_version": "sysml-session-v1", ... }
//! { "type": "tick",      "delta": { ... } }
//! { "type": "verdict",   "verdicts": { ... } }
//! { "type": "completed", "tick": 42, "time_ms": 42000.0 }
//! { "type": "error",     "code": "not_found", "message": "..." }
//! ```
//!

use crate::aggregates::VerdictRollup;
use crate::snapshot_diff::DeltaFrame;
use crate::snapshot_view::NormalizedSnapshot;

/// Subprotocol identifier / schema version. Bump when the frame shape
/// changes in a breaking way; add new `type` variants additively.
pub const SCHEMA_VERSION: &str = "sysml-session-v1";

/// One frame on the session event stream.
///
/// Uses `#[serde(tag = "type")]` for externally-tagged JSON so a client
/// can dispatch on the `type` field without deserializing the whole
/// payload twice.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(tag = "type", rename_all = "lowercase")
)]
pub enum SessionFrame {
    /// Opening frame. Always the first message on a new subscription.
    /// `base` is the snapshot the subsequent deltas apply to.
    Hello {
        /// Schema / subprotocol version — `"sysml-session-v1"`.
        schema_version: String,
        /// Session id this stream is attached to.
        session_id: String,
        /// Tick number of `base`.
        tick: u64,
        /// Simulation time (ms) at `base`.
        time_ms: f64,
        /// Baseline snapshot the client folds future deltas into.
        base: NormalizedSnapshot,
    },
    /// Per-tick delta; the client applies it to its local snapshot.
    Tick {
        /// Delta from the prior frame. The `tick` / `time_ms` inside
        /// `delta` identify the new frame.
        delta: DeltaFrame,
    },
    /// Verdict rollup update. Emitted whenever the rollup differs from
    /// the previously-sent one; elided when unchanged.
    Verdict { tick: u64, verdicts: VerdictRollup },
    /// Session has finished all subsystems / completed its run.
    Completed { tick: u64, time_ms: f64 },
    /// Terminal error. Server closes the socket immediately after.
    Error {
        /// Stable error code (machine-readable).
        code: String,
        /// Human-readable detail.
        message: String,
    },
}

/// Stable error codes used on `SessionFrame::Error`. Treat as `&'static str`
/// so clients can `switch` / `match` on them.
pub mod error_codes {
    /// Session id not found.
    pub const NOT_FOUND: &str = "not_found";
    /// Server cannot project the snapshot (panic / internal failure).
    pub const PROJECTION_FAILED: &str = "projection_failed";
    /// Client's `?since=` cursor is outside the server's replay ring.
    /// The client should reopen without `since` to get a fresh Hello.
    pub const CURSOR_EXPIRED: &str = "cursor_expired";
    /// Server shutting down / session reaped.
    pub const SESSION_CLOSED: &str = "session_closed";
}

impl SessionFrame {
    /// Build a Hello frame for `base`.
    pub fn hello(session_id: impl Into<String>, base: NormalizedSnapshot) -> Self {
        SessionFrame::Hello {
            schema_version: SCHEMA_VERSION.to_string(),
            session_id: session_id.into(),
            tick: base.tick,
            time_ms: base.time_ms,
            base,
        }
    }

    /// Build a Tick frame from a delta.
    pub fn tick(delta: DeltaFrame) -> Self {
        SessionFrame::Tick { delta }
    }

    /// Build a Verdict frame.
    pub fn verdict(tick: u64, verdicts: VerdictRollup) -> Self {
        SessionFrame::Verdict { tick, verdicts }
    }

    /// Build a Completed frame.
    pub fn completed(tick: u64, time_ms: f64) -> Self {
        SessionFrame::Completed { tick, time_ms }
    }

    /// Build an Error frame.
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        SessionFrame::Error {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Short textual tag — useful for telemetry without cloning the whole
    /// frame.
    pub fn tag(&self) -> &'static str {
        match self {
            SessionFrame::Hello { .. } => "hello",
            SessionFrame::Tick { .. } => "tick",
            SessionFrame::Verdict { .. } => "verdict",
            SessionFrame::Completed { .. } => "completed",
            SessionFrame::Error { .. } => "error",
        }
    }
}

#[cfg(all(test, feature = "serde"))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::snapshot_diff::diff;
    use crate::snapshot_view::NormalizedSnapshot;

    fn base_snapshot(tick: u64, time_ms: f64) -> NormalizedSnapshot {
        let mut s = NormalizedSnapshot {
            tick,
            time_ms,
            ..Default::default()
        };
        s.scalar_vars.insert("T_busbar".into(), 320.5);
        s
    }

    #[test]
    fn tag_reports_variant_name() {
        assert_eq!(
            SessionFrame::hello("sess-1", base_snapshot(0, 0.0)).tag(),
            "hello",
        );
        assert_eq!(SessionFrame::completed(5, 500.0).tag(), "completed");
        assert_eq!(SessionFrame::error("not_found", "x").tag(), "error");
    }

    #[test]
    fn hello_frame_includes_schema_version() {
        let f = SessionFrame::hello("sess-42", base_snapshot(3, 30.0));
        let json = serde_json::to_value(&f).unwrap();
        assert_eq!(json["type"], "hello");
        assert_eq!(json["schema_version"], SCHEMA_VERSION);
        assert_eq!(json["session_id"], "sess-42");
        assert_eq!(json["tick"], 3);
        assert_eq!(json["base"]["tick"], 3);
    }

    #[test]
    fn tick_frame_carries_delta() {
        let prev = base_snapshot(0, 0.0);
        let mut next = base_snapshot(1, 10.0);
        next.scalar_vars.insert("T_busbar".into(), 325.0);
        let delta = diff(Some(&prev), &next);
        let f = SessionFrame::tick(delta);
        let json = serde_json::to_value(&f).unwrap();
        assert_eq!(json["type"], "tick");
        assert_eq!(json["delta"]["tick"], 1);
        assert_eq!(
            json["delta"]["scalar_changed"]["T_busbar"]
                .as_f64()
                .unwrap(),
            325.0,
        );
    }

    #[test]
    fn completed_and_error_frames_round_trip() {
        let done = SessionFrame::completed(100, 10_000.0);
        let round: SessionFrame =
            serde_json::from_str(&serde_json::to_string(&done).unwrap()).unwrap();
        assert_eq!(round, done);

        let err = SessionFrame::error(error_codes::NOT_FOUND, "no such session");
        let round: SessionFrame =
            serde_json::from_str(&serde_json::to_string(&err).unwrap()).unwrap();
        assert_eq!(round, err);
    }

    #[test]
    fn verdict_frame_shape() {
        let rollup = VerdictRollup {
            pass: 3,
            fail: 1,
            inconclusive: 0,
            error: 0,
        };
        let f = SessionFrame::verdict(7, rollup);
        let json = serde_json::to_value(&f).unwrap();
        assert_eq!(json["type"], "verdict");
        assert_eq!(json["tick"], 7);
        assert_eq!(json["verdicts"]["pass"], 3);
        assert_eq!(json["verdicts"]["fail"], 1);
    }

    #[test]
    fn error_codes_are_stable_strings() {
        // These are exposed to clients — assert the exact values so an
        // accidental rename shows up as a test failure.
        assert_eq!(error_codes::NOT_FOUND, "not_found");
        assert_eq!(error_codes::PROJECTION_FAILED, "projection_failed");
        assert_eq!(error_codes::CURSOR_EXPIRED, "cursor_expired");
        assert_eq!(error_codes::SESSION_CLOSED, "session_closed");
    }
}
