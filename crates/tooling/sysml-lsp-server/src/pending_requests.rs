//! Request deduplication for expensive LSP handlers.
//!
//! When the editor fires rapid requests for the same document (e.g. completion
//! on every keystroke), only the latest request's result should be returned.
//! Earlier requests that finish after a newer one has started are stale and
//! their results should be discarded.

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

use std::sync::atomic::{AtomicU64, Ordering};

use dashmap::DashMap;

/// Tracks in-flight requests per (URI, capability) pair using generation counters.
///
/// Each call to [`begin`] increments the generation for a given key. After
/// the handler finishes, the caller checks [`is_current`] to determine
/// whether a newer request has superseded it.
pub(crate) struct PendingRequests {
    generations: DashMap<(&'static str, String), u64>,
    next_gen: AtomicU64,
}

/// Token returned by [`PendingRequests::begin`].
///
/// Hold this while the handler runs, then call [`PendingRequests::is_current`]
/// before returning the result.
pub(crate) struct RequestGuard {
    pub capability: &'static str,
    pub uri: String,
    pub generation: u64,
}

impl PendingRequests {
    pub fn new() -> Self {
        Self {
            generations: DashMap::new(),
            next_gen: AtomicU64::new(1),
        }
    }

    /// Register a new request for `(uri, capability)`.
    ///
    /// Returns a guard whose generation can be checked after the handler completes.
    pub fn begin(&self, uri: String, capability: &'static str) -> RequestGuard {
        let gen = self.next_gen.fetch_add(1, Ordering::Relaxed);
        self.generations.insert((capability, uri.clone()), gen);
        RequestGuard {
            capability,
            uri,
            generation: gen,
        }
    }

    /// Check whether `guard` is still the latest request for its key.
    ///
    /// Returns `true` if no newer request has been registered since `begin`.
    pub fn is_current(&self, guard: &RequestGuard) -> bool {
        self.generations
            .get(&(guard.capability, guard.uri.clone()))
            .map(|current| *current == guard.generation)
            .unwrap_or(false)
    }

    /// Clean up the entry after a request completes (only if still current).
    pub fn end(&self, guard: &RequestGuard) {
        // Only remove if we're still the current generation to avoid racing
        // with a newer request that just called begin().
        self.generations
            .remove_if(&(guard.capability, guard.uri.clone()), |_, gen| {
                *gen == guard.generation
            });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn single_request_is_current() {
        let pr = PendingRequests::new();
        let guard = pr.begin("file:///a.sysml".into(), "completion");
        assert!(pr.is_current(&guard));
        pr.end(&guard);
    }

    #[test]
    fn newer_request_supersedes_older() {
        let pr = PendingRequests::new();
        let g1 = pr.begin("file:///a.sysml".into(), "completion");
        let g2 = pr.begin("file:///a.sysml".into(), "completion");
        assert!(!pr.is_current(&g1));
        assert!(pr.is_current(&g2));
        pr.end(&g1); // should not remove because g2 is current
        assert!(pr.is_current(&g2));
        pr.end(&g2);
    }

    #[test]
    fn different_uris_are_independent() {
        let pr = PendingRequests::new();
        let g1 = pr.begin("file:///a.sysml".into(), "completion");
        let g2 = pr.begin("file:///b.sysml".into(), "completion");
        assert!(pr.is_current(&g1));
        assert!(pr.is_current(&g2));
    }

    #[test]
    fn different_capabilities_are_independent() {
        let pr = PendingRequests::new();
        let g1 = pr.begin("file:///a.sysml".into(), "completion");
        let g2 = pr.begin("file:///a.sysml".into(), "semantic_tokens");
        assert!(pr.is_current(&g1));
        assert!(pr.is_current(&g2));
    }
}
