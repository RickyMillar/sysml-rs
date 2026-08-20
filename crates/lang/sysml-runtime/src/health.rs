//! Canonical set of simple graph-level health checks.
//!
//! [`GRAPH_HEALTH_FNS`] is the single source of truth for the "simple
//! graph-level" health diagnostics — each is a plain
//! `fn(&ModelGraph) -> Vec<Diagnostic>` that runs against one per-file graph
//! with no extra routing context. Both the production diagnostic pipeline
//! (`sysml-service`'s `compute_pipeline`) and the test-only `diagnose_source`
//! pipeline (`sysml-lsp-server`) iterate this exact array, so the two cannot
//! silently drift apart.
//!
//! Flow health, import health, and physics health are deliberately **excluded**
//! from this set: they require workspace-merged-graph (or salsa-cached) routing
//! that differs per pipeline, so each pipeline wires them up separately against
//! the graph that is correct for its context.

use sysml_core::ModelGraph;
use sysml_span::Diagnostic;

/// The canonical set of simple graph-level health-diagnostic functions shared
/// by every diagnostic pipeline.
///
/// Each entry takes a single per-file [`ModelGraph`] and returns its
/// diagnostics; ordering is preserved across both consuming pipelines.
///
/// Excludes `flow_health_diagnostics`, import health, and
/// `physics_health_diagnostics` — those need workspace-merged-graph routing
/// that differs per pipeline and stay wired separately at each call site.
pub const GRAPH_HEALTH_FNS: &[fn(&ModelGraph) -> Vec<Diagnostic>] = &[
    crate::statemachine::state_machine_health_diagnostics,
    crate::actions::action_health_diagnostics,
    crate::flows::port_health_diagnostics_from_graph,
    crate::cases::verification_health_diagnostics,
    crate::constraints::constraint_health_diagnostics,
    crate::cases::requirement_health_diagnostics,
    crate::quantity_health::quantity_mismatch_health_diagnostics,
    crate::quantity_health::quantity_expression_health_diagnostics,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_health_fns_has_eight_entries() {
        // Cheap guard against silent shrinkage: if a check is dropped from the
        // canonical set, this fails loudly rather than quietly losing coverage.
        assert_eq!(GRAPH_HEALTH_FNS.len(), 8);
    }
}
