//! Semantic validation check functions for SysML v2 models.
//!
//! Each submodule provides check functions called by the generated
//! semantic validation dispatcher. Check functions take an `Element`
//! and `ModelGraph` reference and return `Option<Vec<SemanticError>>`.
//!
//! ## Convention
//!
//! Each check function has the signature:
//! ```ignore
//! pub fn check_name(element: &Element, graph: &ModelGraph) -> Option<Vec<SemanticError>>
//! ```
//!
//! Returns `None` or `Some(vec![])` if the check passes,
//! `Some(vec![error, ...])` if violations are found.

pub mod actions;
pub mod cardinality;
pub mod composite;
pub mod connectors;
pub mod distinguishability;
pub mod ownership;
pub mod ports;
pub mod requirements;
pub mod specialization;
pub mod states;
pub mod typing;
pub mod variation;
