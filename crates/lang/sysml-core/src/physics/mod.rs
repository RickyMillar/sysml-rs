//! Physics-aware static analysis layer.
//!
//! Classifies ports, attributes, and types by their ISQ (International System of
//! Quantities) dimension vectors, maps them to physics domains (electrical, thermal,
//! hydraulic, mechanical, chemical, luminous), and provides health diagnostics for
//! domain mismatches, direction conflicts, and conservation imbalances.
//!
//! This module contains the **static analysis** portion of the physics layer —
//! everything that can be determined from the ModelGraph alone, without execution
//! state. The companion execution layer (solvers, sweep, executor) lives in
//! `sysml-runtime::physics`.
//!

pub mod classify;
pub mod dimension;
pub mod domain;
pub mod health;
pub mod isq_types;

pub use classify::{ClassificationConfidence, ClassifiedFeature, PortClassification};
pub use dimension::DimensionVector;
pub use domain::{
    BondGraphRole, ConservationLaw, PhysicsDomain, PhysicsDomainRegistry, VariableRole,
};
pub use isq_types::{IsqCategory, IsqTypeEntry, ISQ_TYPES};
