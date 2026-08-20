//! # Advanced ODE solvers
//!
//! This module hosts implicit / specialized integrators that sit alongside the
//! flat explicit solvers ([`crate::ode::Rk4Solver`], [`crate::ode45::Rk45Solver`]).
//!
//! ## Available solvers
//!
//! - [`bdf::BdfSolver`] — variable-order Backward Differentiation Formula
//!   (BDF1–BDF5) for stiff systems. Uses Newton iteration with a dense
//!   finite-difference Jacobian and a PI step-size controller.
//!
//! Explicit (non-stiff) integrators continue to live in the crate-root
//! `ode.rs` / `ode45.rs` modules; we do not re-home them here to keep the
//! public API of `sysml-runtime` stable for existing consumers.

pub mod bdf;

pub use bdf::{detect_stiffness, BdfSolver, BdfStats};
