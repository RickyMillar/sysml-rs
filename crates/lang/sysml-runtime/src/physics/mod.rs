//! Physics-aware simulation layer.
//!
//! Automatically infers physical domains (electrical, thermal, hydraulic, mechanical)
//! from ISQ dimension vectors, generates conservation constraints from flow topology,
//! and provides pluggable domain solvers.
//!
//! Static analysis types (ISQ classification, dimension vectors, domain registry,
//! port classification) are defined in `sysml_core::physics` and re-exported here.
//! Execution types (solvers, sweep, executor) are owned by this crate.
//!

// --- Static analysis (re-exported from sysml-core) ---
pub use sysml_core::physics::classify;
pub use sysml_core::physics::dimension;
pub use sysml_core::physics::domain;
pub use sysml_core::physics::isq_types;

// --- Execution (owned by sysml-runtime) ---
pub mod causality;
pub mod connection;
pub mod constraints;
pub mod dae;
pub mod executor;
pub mod health;
pub mod solver;
pub mod sweep;

pub use classify::{ClassificationConfidence, ClassifiedFeature, PortClassification};
pub use connection::ConnectionGraph;
pub use constraints::{
    generate_constraints_with_model, ConservationConstraint, ConstitutiveRelation, EffortEquality,
    FlowEquality, GeneratedConstraints,
};
pub use dimension::DimensionVector;
pub use domain::{
    BondGraphRole, ConservationLaw, PhysicsDomain, PhysicsDomainRegistry, VariableRole,
};
pub use executor::PhysicsExecutor;
pub use solver::{DomainSolver, PhysicsSolverError, SolveResult};
pub use sweep::RadialSweepSolver;

pub use causality::{assign_causality, Causality, CausalityAssignment};
pub use dae::{BondGraphDae, DaeError, DaeSolution, StateVectorMap};
