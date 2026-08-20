//! Phase 6 — Pre-simulation physics health diagnostics.
//!
//! Provides a quick check function for editors and CLI to report physics
//! topology issues before simulation starts.

use sysml_core::ModelGraph;
use sysml_span::Diagnostic;

use super::executor::PhysicsExecutor;

/// Run physics health diagnostics on a model graph.
///
/// Attempts to build a [`PhysicsExecutor`] from the graph and collects
/// all diagnostics. Returns informational diagnostics if no physics topology
/// is found, or warnings/errors for topology issues.
pub fn physics_health_diagnostics(graph: &ModelGraph) -> Vec<Diagnostic> {
    match PhysicsExecutor::from_graph(graph) {
        Ok((_executor, diags)) => diags,
        Err(diags) => diags,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use sysml_core::ModelGraph;

    /// Empty graph produces informational diagnostics (no flows).
    #[test]
    fn empty_graph_diagnostics() {
        let graph = ModelGraph::new();
        let diags = physics_health_diagnostics(&graph);
        assert!(!diags.is_empty(), "should have at least one diagnostic");
        // Should mention no flow connections
        let msg = &diags[0].message;
        assert!(
            msg.contains("flow") || msg.contains("physics"),
            "diagnostic should mention flows or physics: {:?}",
            msg
        );
    }
}
