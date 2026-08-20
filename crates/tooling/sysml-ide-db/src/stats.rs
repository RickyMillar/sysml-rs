//! Salsa query execution statistics.
//!
//! Provides structured metrics about salsa query effectiveness: cache hits
//! (validated memoized values), cache misses (actual executions), and totals.
//! These counters are updated automatically via the `salsa_event` callback.

use std::sync::atomic::{AtomicU64, Ordering};

/// Atomic counters for salsa query execution events.
///
/// Shared via `Arc` between the `RootDatabase` and its event handler closure.
/// All operations use `Relaxed` ordering since exact precision isn't required
/// for observability counters.
#[derive(Debug, Default)]
pub struct QueryStats {
    /// Number of tracked function executions (cache misses).
    pub executions: AtomicU64,
    /// Number of validated memoized values (cache hits).
    pub validations: AtomicU64,
}

impl QueryStats {
    /// Take a point-in-time snapshot of all counters.
    pub fn snapshot(&self) -> QueryStatsSnapshot {
        QueryStatsSnapshot {
            executions: self.executions.load(Ordering::Relaxed),
            validations: self.validations.load(Ordering::Relaxed),
        }
    }

    /// Reset all counters to zero.
    pub fn reset(&self) {
        self.executions.store(0, Ordering::Relaxed);
        self.validations.store(0, Ordering::Relaxed);
    }
}

/// Immutable snapshot of query statistics at a point in time.
#[derive(Debug, Clone)]
pub struct QueryStatsSnapshot {
    /// Number of tracked function executions (cache misses).
    pub executions: u64,
    /// Number of validated memoized values (cache hits).
    pub validations: u64,
}

impl QueryStatsSnapshot {
    /// Cache hit ratio: validations / (validations + executions).
    /// Returns 0.0 if no queries have been recorded.
    pub fn hit_ratio(&self) -> f64 {
        let total = self.validations + self.executions;
        if total == 0 {
            0.0
        } else {
            self.validations as f64 / total as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_snapshot() {
        let stats = QueryStats::default();
        stats.executions.fetch_add(5, Ordering::Relaxed);
        stats.validations.fetch_add(15, Ordering::Relaxed);

        let snap = stats.snapshot();
        assert_eq!(snap.executions, 5);
        assert_eq!(snap.validations, 15);
        assert!((snap.hit_ratio() - 0.75).abs() < 0.001);
    }

    #[test]
    fn stats_reset() {
        let stats = QueryStats::default();
        stats.executions.fetch_add(10, Ordering::Relaxed);
        stats.reset();

        let snap = stats.snapshot();
        assert_eq!(snap.executions, 0);
        assert_eq!(snap.validations, 0);
    }

    #[test]
    fn hit_ratio_zero_queries() {
        let snap = QueryStatsSnapshot {
            executions: 0,
            validations: 0,
        };
        assert_eq!(snap.hit_ratio(), 0.0);
    }
}
