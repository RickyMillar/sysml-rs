//! View generator implementations.
//!
//! Each generator implements the `ViewGenerator` trait and produces `DiagramIR`.
//! Generators are migrated here one-by-one from `earlier renderer module`.
//!
//! Migration order (simplest → most complex):
//! 1. Browser    — Pure tree, no edges  ✓
//! 2. Grid       — Fixed layout, no edges  ✓
//! 3. Geometry   — Like General with coordinates  ✓
//! 4. General    — Reference implementation  ✓
//! 5. Interconnection — Port inheritance
//! 6. State      — Recursive type expansion
//! 7. Action     — Runtime IR compilation
//! 8. Sequence   — Precomputed routes

pub mod action;
pub mod browser;
pub(crate) mod container;
pub mod general;
pub mod geometry;
pub mod grid;
pub mod interconnection;
pub mod sequence;
pub mod state;
