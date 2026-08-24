//! Diagram Intermediate Representation.
//!
//! The IR layer decouples "what should appear in the diagram" (generators)
//! from "how it renders in retired graph-renderer" (render). This enables:
//! - Testing generator logic without JSON serialization
//! - Swapping renderers (retired graph-renderer, SVG, etc.) without touching generators
//! - Compile-time enforcement via the `ViewGenerator` trait

pub(crate) mod consistency;
pub mod expression_pretty;
pub(crate) mod port_placement;
pub mod generator;
pub mod generators;
pub mod overlays;
pub mod rendering_hints;
pub mod types;

pub use generator::{get_generator, GeneratorContext, ViewGenerator};
pub use overlays::{apply_all as apply_overlays, DiagramOverlay, OverlayList};
pub use rendering_hints::RenderingHints;
pub use types::*;
