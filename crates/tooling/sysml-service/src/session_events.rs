//! Session-event shaping pulled out of the `sysml-api` WebSocket handler.
//!
//! The frontend session-event stream emits typed [`SessionFrame`]s
//! (`Hello` → `Tick`/`Verdict`/`Completed` → optional `Error`). Stages 4+
//! kept the shaping logic inline in `sysml-api/src/session_ws.rs`, which
//! meant the transport layer reached into `sysml-runtime` directly to call
//! `normalize`, `verdict_rollup_from_constraints`, and `diff` against
//! every observed snapshot.
//!
//! S2.T14 collapses that surface into the service so transports stay
//! thin: a [`SessionEventShaper`] holds the per-connection shaping state
//! (last-seen normalized snapshot, last tick, last verdict rollup) and
//! turns raw [`ExecutionSnapshot`]s into ready-to-send `SessionFrame`s.
//! The WS handler keeps responsibility for socket lifecycle, the wire
//! encoding (JSON vs CBOR), and the broadcast/poll switch — but every
//! call into `sysml-runtime` now lives behind this module.

use std::sync::Arc;

use sysml_runtime::aggregates::{verdict_rollup_from_constraints, VerdictRollup};
use sysml_runtime::orchestrator::ExecutionSnapshot;
use sysml_runtime::session_events::{error_codes, SessionFrame};
use sysml_runtime::snapshot_diff::diff;
use sysml_runtime::snapshot_view::{normalize, NormalizedSnapshot};

use crate::SysmlService;

/// Per-connection shaping state. Translates raw runtime snapshots into
/// session-event frames (`Tick`, `Verdict`, `Completed`) ready for the
/// WS layer to encode and forward.
#[derive(Debug, Clone)]
pub struct SessionEventShaper {
    prev: NormalizedSnapshot,
    last_tick: u64,
    verdicts_prev: VerdictRollup,
}

/// Result of opening a fresh session-event stream.
///
/// `frames` is drained by the caller in order. When `shaper` is `Some`,
/// the stream is live and the caller proceeds into its push/poll loop.
/// When `shaper` is `None`, the stream is terminal — the caller sends
/// `frames` (a `Hello` followed optionally by `Completed`, or a single
/// `Error` frame) and closes.
#[derive(Debug)]
pub struct SessionEventOpen {
    pub frames: Vec<SessionFrame>,
    pub shaper: Option<SessionEventShaper>,
}

/// Result of one fallback poll. Mirrors the shape the WS handler used
/// before the migration so the call-site stays readable.
#[derive(Debug)]
pub enum SessionEventPoll {
    /// Tick advanced (or session completed). Caller sends each frame
    /// in order; `Completed` is the last one when present.
    Frames(Vec<SessionFrame>),
    /// Tick unchanged; nothing to emit.
    NoChange,
    /// Session disappeared (reaped / stopped externally). Caller emits
    /// an [`error_codes::SESSION_CLOSED`] error frame and closes.
    SessionGone,
    /// Service returned an error on read. Caller emits an
    /// [`error_codes::PROJECTION_FAILED`] error frame and closes.
    Failed(String),
}

impl SessionEventShaper {
    /// Open a new stream against `session_id`. Returns the initial frame
    /// sequence (`Hello`, plus `Completed` if the session is already
    /// done) and a live shaper, or a terminal `Error` frame.
    ///
    /// Callers are expected to forward `frames` verbatim before either
    /// entering their live loop (when `shaper.is_some()`) or closing.
    pub fn open(service: &SysmlService, session_id: &str) -> SessionEventOpen {
        let detail = match service.sessions_info(session_id, Some(true)) {
            Ok(Some(d)) => d,
            Ok(None) => {
                return SessionEventOpen {
                    frames: vec![SessionFrame::error(error_codes::NOT_FOUND, "no such session")],
                    shaper: None,
                };
            }
            Err(e) => {
                return SessionEventOpen {
                    frames: vec![SessionFrame::error(
                        error_codes::PROJECTION_FAILED,
                        e.to_string(),
                    )],
                    shaper: None,
                };
            }
        };

        let Some(raw) = detail.latest_snapshot else {
            return SessionEventOpen {
                frames: vec![SessionFrame::error(
                    error_codes::PROJECTION_FAILED,
                    "session has no snapshot yet",
                )],
                shaper: None,
            };
        };

        let prev = normalize(&raw);
        let mut frames = Vec::with_capacity(2);
        frames.push(SessionFrame::hello(session_id, prev.clone()));

        if prev.completed {
            frames.push(SessionFrame::completed(prev.tick, prev.time_ms));
            return SessionEventOpen {
                frames,
                shaper: None,
            };
        }

        let verdicts_prev = verdict_rollup_from_constraints(&prev.constraint_results);
        let shaper = SessionEventShaper {
            last_tick: prev.tick,
            prev,
            verdicts_prev,
        };
        SessionEventOpen {
            frames,
            shaper: Some(shaper),
        }
    }

    /// Push path. Called when the broadcast channel delivers a fresh
    /// snapshot. Returns the frames the caller should send for this
    /// transition (may be empty when nothing relevant changed).
    pub fn on_snapshot(&mut self, snapshot: &ExecutionSnapshot) -> Vec<SessionFrame> {
        let next = normalize(snapshot);
        self.advance(next)
    }

    /// Fallback poll path. Used after a `Lagged` broadcast event or when
    /// the session predates the broadcast subscription. Mirrors the
    /// behaviour of the legacy `poll_once` helper.
    pub fn poll(&mut self, service: &SysmlService, session_id: &str) -> SessionEventPoll {
        let (current_tick, completed) = match service.session_pulse(session_id) {
            Some(pulse) => pulse,
            None => return SessionEventPoll::SessionGone,
        };
        if current_tick == self.last_tick && !completed {
            return SessionEventPoll::NoChange;
        }

        match service.sessions_info(session_id, Some(true)) {
            Ok(Some(detail)) => {
                let Some(raw) = detail.latest_snapshot else {
                    return SessionEventPoll::NoChange;
                };
                let next = normalize(&raw);
                if next.tick == self.last_tick && !next.completed {
                    return SessionEventPoll::NoChange;
                }
                SessionEventPoll::Frames(self.advance(next))
            }
            Ok(None) => SessionEventPoll::SessionGone,
            Err(e) => SessionEventPoll::Failed(e.to_string()),
        }
    }

    /// Common shaping for both the push and poll paths. Mutates the
    /// stored state (last_tick, prev, verdicts_prev) in lockstep with
    /// the emitted frames so callers can't observe a half-applied state.
    fn advance(&mut self, next: NormalizedSnapshot) -> Vec<SessionFrame> {
        let mut frames: Vec<SessionFrame> = Vec::new();
        if next.tick != self.last_tick {
            let d = diff(Some(&self.prev), &next);
            frames.push(SessionFrame::tick(d));
        }
        let verdicts_next = verdict_rollup_from_constraints(&next.constraint_results);
        if verdicts_next != self.verdicts_prev {
            frames.push(SessionFrame::verdict(next.tick, verdicts_next.clone()));
        }
        if next.completed {
            frames.push(SessionFrame::completed(next.tick, next.time_ms));
        }
        self.last_tick = next.tick;
        self.prev = next;
        self.verdicts_prev = verdicts_next;
        frames
    }
}

/// Convenience: take an `Arc<ExecutionSnapshot>` straight off the
/// broadcast receiver. Same as [`SessionEventShaper::on_snapshot`] —
/// just spares the WS layer the manual deref.
pub fn ingest_arc(
    shaper: &mut SessionEventShaper,
    snapshot: &Arc<ExecutionSnapshot>,
) -> Vec<SessionFrame> {
    shaper.on_snapshot(snapshot.as_ref())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use sysml_runtime::cases::VerdictKind;
    use sysml_runtime::snapshot_view::ConstraintView;

    fn snap_with(tick: u64, time_ms: f64, completed: bool) -> NormalizedSnapshot {
        NormalizedSnapshot {
            tick,
            time_ms,
            completed,
            ..Default::default()
        }
    }

    fn constraint(name: &str, satisfied: bool) -> ConstraintView {
        ConstraintView {
            name: name.into(),
            verdict: if satisfied { VerdictKind::Pass } else { VerdictKind::Fail },
            ..Default::default()
        }
    }

    fn shaper_at(prev: NormalizedSnapshot) -> SessionEventShaper {
        let verdicts_prev = verdict_rollup_from_constraints(&prev.constraint_results);
        SessionEventShaper {
            last_tick: prev.tick,
            prev,
            verdicts_prev,
        }
    }

    #[test]
    fn advance_emits_tick_when_tick_increases() {
        let mut shaper = shaper_at(snap_with(1, 100.0, false));
        let next = snap_with(2, 200.0, false);
        let frames = shaper.advance(next);
        assert_eq!(frames.len(), 1);
        assert!(matches!(frames[0], SessionFrame::Tick { .. }));
        assert_eq!(shaper.last_tick, 2);
    }

    #[test]
    fn advance_emits_verdict_when_rollup_changes() {
        let mut shaper = shaper_at(snap_with(1, 100.0, false));
        let mut next = snap_with(2, 200.0, false);
        next.constraint_results.push(constraint("c1", true));
        let frames = shaper.advance(next);
        // Tick + Verdict, in that order.
        assert_eq!(frames.len(), 2);
        assert!(matches!(frames[0], SessionFrame::Tick { .. }));
        assert!(matches!(frames[1], SessionFrame::Verdict { .. }));
    }

    #[test]
    fn advance_emits_completed_when_session_finishes() {
        let mut shaper = shaper_at(snap_with(1, 100.0, false));
        let next = snap_with(2, 200.0, true);
        let frames = shaper.advance(next);
        // Tick + Completed.
        assert_eq!(frames.len(), 2);
        assert!(matches!(frames.last(), Some(SessionFrame::Completed { .. })));
    }

    #[test]
    fn advance_emits_nothing_for_unchanged_tick() {
        let mut shaper = shaper_at(snap_with(5, 500.0, false));
        let frames = shaper.advance(snap_with(5, 500.0, false));
        assert!(frames.is_empty());
    }

    #[test]
    fn advance_updates_state_in_lockstep_with_frames() {
        let mut shaper = shaper_at(snap_with(1, 100.0, false));
        let _ = shaper.advance(snap_with(2, 200.0, false));
        // A second invocation against the same payload must be a no-op
        // — this is the contract the WS layer relied on when it cached
        // `last_tick` after each successful send.
        let frames = shaper.advance(snap_with(2, 200.0, false));
        assert!(frames.is_empty());
    }

    #[test]
    fn open_emits_error_for_missing_session() {
        let service = SysmlService::empty();
        let out = SessionEventShaper::open(&service, "no-such-id");
        assert!(out.shaper.is_none());
        assert_eq!(out.frames.len(), 1);
        match &out.frames[0] {
            SessionFrame::Error { code, .. } => assert_eq!(code, error_codes::NOT_FOUND),
            other => panic!("expected error frame, got {other:?}"),
        }
    }

    // Snapshot-rollup parity: identical satisfied counts produce no
    // Verdict frame.
    #[test]
    fn advance_keeps_rollup_steady_when_constraints_unchanged() {
        let mut prev = snap_with(0, 0.0, false);
        prev.constraint_results.push(constraint("c1", true));
        let mut shaper = shaper_at(prev);
        let mut next = snap_with(1, 100.0, false);
        next.constraint_results.push(constraint("c1", true));
        let frames = shaper.advance(next);
        assert_eq!(frames.len(), 1);
        assert!(matches!(frames[0], SessionFrame::Tick { .. }));
    }

    // A satisfied → unsatisfied flip moves the rollup, so a Verdict
    // frame must follow the Tick.
    #[test]
    fn advance_emits_verdict_when_satisfied_flips() {
        let mut prev = snap_with(0, 0.0, false);
        prev.constraint_results.push(constraint("c1", true));
        let mut shaper = shaper_at(prev);
        let mut next = snap_with(1, 100.0, false);
        next.constraint_results.push(constraint("c1", false));
        let frames = shaper.advance(next);
        assert_eq!(frames.len(), 2);
        assert!(matches!(frames[0], SessionFrame::Tick { .. }));
        assert!(matches!(frames[1], SessionFrame::Verdict { .. }));
    }
}
