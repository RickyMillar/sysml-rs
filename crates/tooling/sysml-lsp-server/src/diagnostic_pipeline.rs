//! Diagnostic pipeline: background coordinator and task ID generation.

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

use dashmap::DashMap;
use tokio::task::JoinHandle;

/// Diagnostic pipeline: task ID generation and per-URI task management.
pub struct DiagnosticPipeline {
    /// Monotonic sequence used to generate background task correlation IDs.
    pub background_task_seq: std::sync::atomic::AtomicU64,
    /// Latest scheduled diagnostics task per document URI.
    diagnostics_tasks: DashMap<String, JoinHandle<()>>,
}

impl DiagnosticPipeline {
    pub fn new() -> Self {
        Self {
            background_task_seq: std::sync::atomic::AtomicU64::new(1),
            diagnostics_tasks: DashMap::new(),
        }
    }

    /// Generate a stable correlation ID for background/spawned tasks.
    pub fn next_background_task_id(&self, prefix: &str) -> String {
        let seq = self
            .background_task_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("{prefix}-{seq}")
    }

    /// Replace the pending diagnostics task for a URI, aborting any prior task.
    pub fn replace_diagnostics_task(&self, uri: String, handle: JoinHandle<()>) {
        if let Some(previous) = self.diagnostics_tasks.insert(uri, handle) {
            previous.abort();
        }
    }

    /// Remove and abort the pending diagnostics task for a URI (if present).
    pub fn cancel_diagnostics_task(&self, uri: &str) {
        if let Some((_, task)) = self.diagnostics_tasks.remove(uri) {
            task.abort();
        }
    }

    /// Clear and abort all pending diagnostics tasks.
    pub fn cancel_all_diagnostics_tasks(&self) {
        let tasks: Vec<_> = self
            .diagnostics_tasks
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        for uri in tasks {
            self.cancel_diagnostics_task(&uri);
        }
    }

    #[cfg(test)]
    pub fn pending_diagnostics_tasks(&self) -> usize {
        self.diagnostics_tasks.len()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn replace_diagnostics_task_aborts_previous_for_same_uri() {
        let pipeline = DiagnosticPipeline::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let uri = "file:///a.sysml".to_string();

        let counter_a = counter.clone();
        let first = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            counter_a.fetch_add(1, Ordering::SeqCst);
        });
        pipeline.replace_diagnostics_task(uri.clone(), first);

        let counter_b = counter.clone();
        let second = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            counter_b.fetch_add(1, Ordering::SeqCst);
        });
        pipeline.replace_diagnostics_task(uri.clone(), second);

        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert_eq!(pipeline.pending_diagnostics_tasks(), 1);
        pipeline.cancel_diagnostics_task(&uri);
    }

    #[tokio::test]
    async fn cancel_all_diagnostics_tasks_aborts_everything() {
        let pipeline = DiagnosticPipeline::new();
        let counter = Arc::new(AtomicUsize::new(0));

        for idx in 0..3 {
            let counter = counter.clone();
            let handle = tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                counter.fetch_add(1, Ordering::SeqCst);
            });
            pipeline.replace_diagnostics_task(format!("file:///{idx}.sysml"), handle);
        }

        pipeline.cancel_all_diagnostics_tasks();
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        assert_eq!(counter.load(Ordering::SeqCst), 0);
        assert_eq!(pipeline.pending_diagnostics_tasks(), 0);
    }
}
