//! Phase 5 — DomainSolver trait.
//!
//! Pluggable solver interface for physical domain subgraphs. Each solver
//! handles a specific domain (electrical, thermal, etc.) and topology.

use super::connection::ConnectionGraph;
use super::constraints::{ConservationConstraint, EffortEquality};
use sysml_span::Diagnostic;

use crate::expressions::EvalContext;

// ---------------------------------------------------------------------------
// Result / Error
// ---------------------------------------------------------------------------

/// Result of a domain solve step.
#[derive(Debug, Clone)]
pub struct SolveResult {
    /// Number of variables that were assigned during this solve.
    pub variables_solved: usize,
    /// Residual error (0.0 for exact solvers on tree topologies).
    pub residual: f64,
}

/// Error during domain solving.
#[derive(Debug, thiserror::Error)]
pub enum PhysicsSolverError {
    /// A general solver failure.
    #[error("solver failed: {0}")]
    Failed(String),
    /// The solver does not support the given topology.
    #[error("topology not supported: {0}")]
    UnsupportedTopology(String),
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Pluggable solver for a physical domain.
///
/// Implementations handle specific topologies (tree, mesh, etc.) and
/// conservation laws. The [`super::sweep::RadialSweepSolver`] is the default
/// solver for tree/radial topologies.
pub trait DomainSolver: Send + Sync {
    /// Which domain this solver handles (e.g., `"electrical"`).
    fn domain(&self) -> &str;

    /// Whether this solver can handle the given subgraph topology.
    fn can_solve(&self, graph: &ConnectionGraph) -> bool;

    /// Solve for unknown effort/flow variables given known values in `ctx`.
    fn solve(
        &self,
        subgraph: &ConnectionGraph,
        constraints: &[ConservationConstraint],
        equalities: &[EffortEquality],
        ctx: &mut EvalContext,
    ) -> Result<SolveResult, PhysicsSolverError>;

    /// Clone into a boxed trait object (required by `Executor::clone_boxed`).
    fn clone_boxed(&self) -> Box<dyn DomainSolver>;

    /// Health diagnostics for this solver.
    fn diagnostics(&self, _graph: &ConnectionGraph) -> Vec<Diagnostic> {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::flows::port::PortDirection;
    use crate::physics::connection::{ConnectionGraph, PhysicsConnection, PhysicsPortNode};
    use crate::physics::sweep::RadialSweepSolver;

    /// Helper: build a PhysicsPortNode.
    fn node(
        id: usize,
        owner: &str,
        port: &str,
        domain: Option<&'static str>,
        dir: PortDirection,
    ) -> PhysicsPortNode {
        PhysicsPortNode {
            id,
            qualified_path: format!("{}.{}", owner, port),
            owner_path: owner.to_string(),
            port_name: port.to_string(),
            domain,
            direction: dir,
            classification: None,
        }
    }

    #[test]
    fn radial_sweep_can_solve_tree() {
        let graph = ConnectionGraph {
            nodes: vec![
                node(0, "a", "out", Some("electrical"), PortDirection::Out),
                node(1, "b", "in", Some("electrical"), PortDirection::In),
                node(2, "b", "out", Some("electrical"), PortDirection::Out),
                node(3, "c", "in", Some("electrical"), PortDirection::In),
            ],
            edges: vec![
                PhysicsConnection {
                    source: 0,
                    target: 1,
                    domain: Some("electrical"),
                    enabled: true,
                },
                PhysicsConnection {
                    source: 2,
                    target: 3,
                    domain: Some("electrical"),
                    enabled: true,
                },
            ],
            junctions: vec![],
        };

        let solver = RadialSweepSolver {
            domain: "electrical".to_string(),
        };
        assert!(solver.can_solve(&graph), "tree topology should be solvable");
    }

    #[test]
    fn radial_sweep_cannot_solve_cycle() {
        let graph = ConnectionGraph {
            nodes: vec![
                node(0, "a", "out", None, PortDirection::Out),
                node(1, "b", "out", None, PortDirection::Out),
                node(2, "c", "out", None, PortDirection::Out),
            ],
            edges: vec![
                PhysicsConnection {
                    source: 0,
                    target: 1,
                    domain: None,
                    enabled: true,
                },
                PhysicsConnection {
                    source: 1,
                    target: 2,
                    domain: None,
                    enabled: true,
                },
                PhysicsConnection {
                    source: 2,
                    target: 0,
                    domain: None,
                    enabled: true,
                }, // cycle
            ],
            junctions: vec![],
        };

        let solver = RadialSweepSolver {
            domain: "electrical".to_string(),
        };
        assert!(
            !solver.can_solve(&graph),
            "cyclic topology should not be solvable by radial sweep"
        );
    }
}
