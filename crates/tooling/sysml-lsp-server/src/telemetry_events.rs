//! Stable telemetry events used for smoke checks and dashboards.

// LSP server: tower-lsp patterns use unwrap/expect for client sends,
// indexing is bounds-checked by protocol invariants, Arc cloning is intentional.
#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::manual_let_else,
    clippy::arc_with_non_send_sync,
    clippy::clone_on_ref_ptr,
    clippy::map_err_ignore,
    clippy::needless_pass_by_value,
    clippy::panic
)]

use crate::telemetry_control;

const LATENCY_SAMPLE_EVERY_N: u64 = 100;
const COMPLETION_PHASE_SAMPLE_EVERY_N: u64 = 50;
const COMPLETION_SLOW_MS: u128 = 30;
const HOVER_SLOW_MS: u128 = 25;

/// Emit an event when a document is opened.
pub(crate) fn did_open(document_uri: &str, bytes: usize) {
    tracing::info!(
        event = "lsp.did_open",
        document_uri,
        bytes,
        "lsp flow event"
    );
}

/// Emit an event when a document is changed.
pub(crate) fn did_change(document_uri: &str, bytes: usize, changes: usize) {
    tracing::info!(
        event = "lsp.did_change",
        document_uri,
        bytes,
        changes,
        "lsp flow event"
    );
}

/// Emit a manifest parse-error telemetry event and increment counters.
pub(crate) fn manifest_parse_error(document_uri: &str, message: &str) {
    let total = telemetry_control::increment_counter("lsp.counter.manifest_parse_errors_total");
    if total == 1 || telemetry_control::should_log_every_n("lsp.manifest.parse_error.sample", 25) {
        tracing::warn!(
            event = "lsp.manifest.parse_error",
            document_uri,
            total,
            message,
            "manifest parse telemetry"
        );
    }
}

// `workspace_discovery_mode`, `workspace_member_expansion`, and
// `dependency_hydration_failure` lived here when `project_discovery`
// was an LSP-only module. They moved to `sysml-service` in S2.T11
// (3/N) — the moved file embeds equivalent `tracing::info!/warn!`
// emissions inline via its own in-module `telemetry_events` shim,
// so the LSP-side counter-increment + rate-limited variants no
// longer have callers. Deleted to clear the dead-code warnings.

/// Emit an event when verification completes.
pub(crate) fn verify_complete(case_name: &str, verdict: &str, passed: usize, total: usize) {
    tracing::info!(
        event = "lsp.verify.complete",
        case_name,
        verdict,
        passed,
        total,
        "lsp flow event"
    );
}

/// Emit sampled completion latency telemetry.
pub(crate) fn completion_latency(
    document_uri: &str,
    route: &str,
    trigger: &str,
    result: &str,
    item_count: usize,
    stale_discarded: bool,
    elapsed_ms: u128,
) {
    let slow = elapsed_ms >= COMPLETION_SLOW_MS;
    if !slow
        && !telemetry_control::should_log_every_n(
            "lsp.completion.latency.sample",
            LATENCY_SAMPLE_EVERY_N,
        )
    {
        return;
    }

    if slow {
        tracing::warn!(
            event = "lsp.completion.latency",
            document_uri,
            route,
            trigger,
            result,
            item_count,
            stale_discarded,
            elapsed_ms,
            "completion latency telemetry"
        );
    } else {
        tracing::info!(
            event = "lsp.completion.latency",
            document_uri,
            route,
            trigger,
            result,
            item_count,
            stale_discarded,
            elapsed_ms,
            "completion latency telemetry"
        );
    }
}

/// Emit sampled completion phase timing telemetry.
pub(crate) fn completion_phases(
    document_uri: &str,
    route: &str,
    trigger: &str,
    result: &str,
    item_count: usize,
    stale_discarded: bool,
    context_us: u128,
    provider_us: u128,
    finalize_us: u128,
    elapsed_ms: u128,
) {
    let slow = elapsed_ms >= COMPLETION_SLOW_MS;
    if !slow
        && !telemetry_control::should_log_every_n(
            "lsp.completion.phases.sample",
            COMPLETION_PHASE_SAMPLE_EVERY_N,
        )
    {
        return;
    }

    if slow {
        tracing::warn!(
            event = "lsp.completion.phases",
            document_uri,
            route,
            trigger,
            result,
            item_count,
            stale_discarded,
            context_us,
            provider_us,
            finalize_us,
            elapsed_ms,
            "completion phase telemetry"
        );
    } else {
        tracing::info!(
            event = "lsp.completion.phases",
            document_uri,
            route,
            trigger,
            result,
            item_count,
            stale_discarded,
            context_us,
            provider_us,
            finalize_us,
            elapsed_ms,
            "completion phase telemetry"
        );
    }
}

/// Emit sampled hover latency telemetry.
pub(crate) fn hover_latency(
    document_uri: &str,
    source: &str,
    result: &str,
    used_nearest: bool,
    elapsed_ms: u128,
) {
    let slow = elapsed_ms >= HOVER_SLOW_MS;
    if !slow
        && !telemetry_control::should_log_every_n(
            "lsp.hover.latency.sample",
            LATENCY_SAMPLE_EVERY_N,
        )
    {
        return;
    }

    if slow {
        tracing::warn!(
            event = "lsp.hover.latency",
            document_uri,
            source,
            result,
            used_nearest,
            elapsed_ms,
            "hover latency telemetry"
        );
    } else {
        tracing::info!(
            event = "lsp.hover.latency",
            document_uri,
            source,
            result,
            used_nearest,
            elapsed_ms,
            "hover latency telemetry"
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use std::sync::{Arc, Mutex, OnceLock};

    use tracing::{field::Visit, Event, Subscriber};
    use tracing_subscriber::layer::{Context, Layer};
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::Registry;

    use super::*;

    #[derive(Default)]
    struct EventCapture {
        names: Arc<Mutex<Vec<String>>>,
    }

    struct CaptureLayer {
        names: Arc<Mutex<Vec<String>>>,
    }

    struct EventVisitor {
        event_name: Option<String>,
    }

    impl Visit for EventVisitor {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            if field.name() == "event" {
                self.event_name = Some(value.to_string());
            }
        }

        fn record_debug(&mut self, _field: &tracing::field::Field, _value: &dyn std::fmt::Debug) {}
    }

    impl<S> Layer<S> for CaptureLayer
    where
        S: Subscriber,
    {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = EventVisitor { event_name: None };
            event.record(&mut visitor);
            if let Some(name) = visitor.event_name {
                self.names
                    .lock()
                    .expect("event capture lock poisoned")
                    .push(name);
            }
        }
    }

    fn telemetry_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn emits_key_flow_events() {
        let _lock = telemetry_test_lock();
        crate::telemetry_control::reset_for_tests();
        let capture = EventCapture::default();
        let layer = CaptureLayer {
            names: capture.names.clone(),
        };
        let subscriber = Registry::default().with(layer);

        let _guard = tracing::subscriber::set_default(subscriber);
        did_open("file:///open.sysml", 12);
        did_change("file:///open.sysml", 18, 1);
        manifest_parse_error("file:///sysml.toml", "expected a table key");
        verify_complete("SpeedCase", "pass", 3, 3);
        completion_latency(
            "file:///open.sysml",
            "general",
            "manual",
            "ok",
            4,
            false,
            35,
        );
        completion_phases(
            "file:///open.sysml",
            "general",
            "manual",
            "ok",
            4,
            false,
            5_000,
            20_000,
            500,
            35,
        );
        hover_latency("file:///open.sysml", "model", "ok", false, 30);

        let names = capture.names.lock().expect("event capture lock poisoned");
        assert!(names.iter().any(|n| n == "lsp.did_open"));
        assert!(names.iter().any(|n| n == "lsp.did_change"));
        assert!(names.iter().any(|n| n == "lsp.manifest.parse_error"));
        assert!(names.iter().any(|n| n == "lsp.verify.complete"));
        assert!(names.iter().any(|n| n == "lsp.completion.latency"));
        assert!(names.iter().any(|n| n == "lsp.completion.phases"));
        assert!(names.iter().any(|n| n == "lsp.hover.latency"));
    }

    #[test]
    fn latency_sampling_logs_first_nth_and_slow_paths() {
        let _lock = telemetry_test_lock();
        crate::telemetry_control::reset_for_tests();
        let capture = EventCapture::default();
        let layer = CaptureLayer {
            names: capture.names.clone(),
        };
        let subscriber = Registry::default().with(layer);

        let _guard = tracing::subscriber::set_default(subscriber);

        for _ in 0..99 {
            completion_latency("file:///open.sysml", "general", "manual", "ok", 4, false, 5);
            completion_phases(
                "file:///open.sysml",
                "general",
                "manual",
                "ok",
                4,
                false,
                2_000,
                2_000,
                200,
                5,
            );
            hover_latency("file:///open.sysml", "model", "ok", false, 5);
        }
        completion_latency("file:///open.sysml", "general", "manual", "ok", 4, false, 5);
        completion_phases(
            "file:///open.sysml",
            "general",
            "manual",
            "ok",
            4,
            false,
            2_000,
            2_000,
            200,
            5,
        );
        completion_latency(
            "file:///open.sysml",
            "general",
            "manual",
            "ok",
            4,
            false,
            35,
        );
        completion_phases(
            "file:///open.sysml",
            "general",
            "manual",
            "ok",
            4,
            false,
            5_000,
            20_000,
            500,
            35,
        );
        hover_latency("file:///open.sysml", "model", "ok", false, 5);
        hover_latency("file:///open.sysml", "model", "ok", false, 30);

        let names = capture.names.lock().expect("event capture lock poisoned");
        let completion_events = names
            .iter()
            .filter(|n| n.as_str() == "lsp.completion.latency")
            .count();
        let hover_events = names
            .iter()
            .filter(|n| n.as_str() == "lsp.hover.latency")
            .count();
        let completion_phase_events = names
            .iter()
            .filter(|n| n.as_str() == "lsp.completion.phases")
            .count();

        // Completion latency: first + 100th + slow-path.
        assert_eq!(completion_events, 3);
        // Completion phases: first + 50th + 100th + slow-path.
        assert_eq!(completion_phase_events, 4);
        // Hover latency: first + 100th + slow-path.
        assert_eq!(hover_events, 3);
    }
}
