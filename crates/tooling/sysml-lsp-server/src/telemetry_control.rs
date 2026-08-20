//! Lightweight telemetry volume controls.
//!
//! These helpers keep high-frequency logs useful by sampling or cooling down
//! repeated events without introducing external dependencies.

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

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
#[cfg(test)]
use std::time::{Duration, Instant};

fn counters() -> &'static Mutex<HashMap<String, u64>> {
    static COUNTERS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    COUNTERS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
fn cooldowns() -> &'static Mutex<HashMap<String, Instant>> {
    static COOLDOWNS: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    COOLDOWNS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_recover<'a, T>(mutex: &'a Mutex<T>, lock_name: &str) -> MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(lock = lock_name, "recovering from poisoned telemetry lock");
            poisoned.into_inner()
        }
    }
}

fn parse_env_bool(var: &str) -> bool {
    match std::env::var(var) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

/// Enables verbose dependency hydration/goto tracing when requested.
///
/// Set `SYSML_DEPENDENCY_TRACE=1` to activate high-signal investigation logs.
pub(crate) fn dependency_trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        parse_env_bool("SYSML_DEPENDENCY_TRACE") || parse_env_bool("SYSML_LSP_DEPENDENCY_TRACE")
    })
}

/// Returns true for the first event and then every `n`th event for the same key.
pub(crate) fn should_log_every_n(key: impl AsRef<str>, n: u64) -> bool {
    if n <= 1 {
        return true;
    }

    let mut map = lock_recover(counters(), "telemetry-counters");
    let counter = map.entry(key.as_ref().to_owned()).or_insert(0);
    *counter = counter.saturating_add(1);
    *counter == 1 || (*counter).is_multiple_of(n)
}

/// Increment a telemetry counter by `delta` and return the new value.
pub(crate) fn add_counter(key: impl AsRef<str>, delta: u64) -> u64 {
    let mut map = lock_recover(counters(), "telemetry-counters");
    let counter = map.entry(key.as_ref().to_owned()).or_insert(0);
    *counter = counter.saturating_add(delta);
    *counter
}

/// Increment a telemetry counter by one and return the new value.
pub(crate) fn increment_counter(key: impl AsRef<str>) -> u64 {
    add_counter(key, 1)
}

/// Return a sorted snapshot of counters, optionally filtered by key prefix.
pub(crate) fn counter_snapshot(prefix: Option<&str>) -> Vec<(String, u64)> {
    let map = lock_recover(counters(), "telemetry-counters");
    let mut entries: Vec<(String, u64)> = map
        .iter()
        .filter(|(k, _)| prefix.map(|p| k.starts_with(p)).unwrap_or(true))
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

/// Returns true if no event for `key` has been logged within `cooldown`.
#[cfg(test)]
pub(crate) fn should_log_after_cooldown(key: impl AsRef<str>, cooldown: Duration) -> bool {
    let now = Instant::now();
    let mut map = lock_recover(cooldowns(), "telemetry-cooldowns");
    match map.get_mut(key.as_ref()) {
        Some(last) if now.saturating_duration_since(*last) < cooldown => false,
        Some(last) => {
            *last = now;
            true
        }
        None => {
            map.insert(key.as_ref().to_string(), now);
            true
        }
    }
}

#[cfg(test)]
pub(crate) fn reset_for_tests() {
    counters()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    cooldowns()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn every_n_logs_first_and_nth() {
        reset_for_tests();
        assert!(should_log_every_n("sample-key", 3));
        assert!(!should_log_every_n("sample-key", 3));
        assert!(should_log_every_n("sample-key", 3));
        assert!(!should_log_every_n("sample-key", 3));
        assert!(!should_log_every_n("sample-key", 3));
        assert!(should_log_every_n("sample-key", 3));
    }

    #[test]
    fn cooldown_suppresses_until_elapsed() {
        reset_for_tests();
        let cooldown = Duration::from_millis(20);
        assert!(should_log_after_cooldown("cooldown-key", cooldown));
        assert!(!should_log_after_cooldown("cooldown-key", cooldown));
        std::thread::sleep(Duration::from_millis(50));
        assert!(should_log_after_cooldown("cooldown-key", cooldown));
    }

    #[test]
    fn counters_can_increment_and_snapshot_with_prefix() {
        reset_for_tests();
        assert_eq!(increment_counter("lsp.counter.alpha"), 1);
        assert_eq!(add_counter("lsp.counter.alpha", 4), 5);
        assert_eq!(increment_counter("lsp.counter.beta"), 1);
        assert_eq!(increment_counter("other.counter"), 1);

        let all = counter_snapshot(None);
        assert!(
            all.iter().any(|(k, v)| k == "lsp.counter.alpha" && *v == 5),
            "expected alpha counter in full snapshot: {all:?}"
        );
        assert!(
            all.iter().any(|(k, v)| k == "lsp.counter.beta" && *v == 1),
            "expected beta counter in full snapshot: {all:?}"
        );
        assert!(
            all.iter().any(|(k, v)| k == "other.counter" && *v == 1),
            "expected non-lsp counter in full snapshot: {all:?}"
        );

        let filtered = counter_snapshot(Some("lsp.counter."));
        assert!(
            filtered
                .iter()
                .any(|(k, v)| k == "lsp.counter.alpha" && *v == 5),
            "expected alpha counter in filtered snapshot: {filtered:?}"
        );
        assert!(
            filtered
                .iter()
                .any(|(k, v)| k == "lsp.counter.beta" && *v == 1),
            "expected beta counter in filtered snapshot: {filtered:?}"
        );
    }
}
