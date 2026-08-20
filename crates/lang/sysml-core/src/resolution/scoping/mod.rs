//! Scoping strategies for name resolution.
//!
//! Different cross-references in SysML v2 require different scoping strategies.
//! This module provides implementations for each strategy.
//!
//! ## Strategy Overview
//!
//! | Strategy | Description | Example Uses |
//! |----------|-------------|--------------|
//! | OwningNamespace | Walk up namespace hierarchy | Specialization, Subsetting |
//! | NonExpressionNamespace | Skip expression scopes | FeatureTyping, Conjugation |
//! | RelativeNamespace | Relative to previous element | Feature chains |
//! | FeatureChaining | Position-dependent chains | FeatureChainExpression |
//! | TransitionSpecific | State machine transitions | Triggers, guards, effects |
//! | Global | Root package scope | Imports |

// Import tracing macro from parent module
#[allow(unused_imports)]
use super::res_trace;

pub mod chaining;
pub mod global;
pub mod non_expression;
pub mod owning;
pub mod relative;
pub mod transition;

pub use chaining::resolve_with_feature_chaining;
pub use global::resolve_in_global_scope;
pub use non_expression::resolve_in_non_expression_namespace;
pub use owning::resolve_in_owning_namespace;
pub use relative::resolve_in_relative_namespace;
pub use transition::resolve_in_transition_context;

use crate::ElementId;
use crate::ModelGraph;

/// Result of a scoped resolution attempt.
#[derive(Debug, Clone)]
pub enum ScopedResolution {
    /// Successfully resolved to an element.
    Found(ElementId),
    /// Name not found in scope.
    NotFound,
    /// Ambiguous - multiple candidates found.
    Ambiguous(Vec<ElementId>),
    /// Error during resolution (e.g., cycle detected).
    Error(String),
}

impl ScopedResolution {
    /// Returns `true` if resolution was successful.
    pub fn is_found(&self) -> bool {
        matches!(self, ScopedResolution::Found(_))
    }

    /// Returns the resolved element ID if found.
    pub fn element_id(&self) -> Option<ElementId> {
        match self {
            ScopedResolution::Found(id) => Some(id.clone()),
            _ => None,
        }
    }

    /// Convert to Option, discarding error information.
    pub fn ok(self) -> Option<ElementId> {
        match self {
            ScopedResolution::Found(id) => Some(id),
            _ => None,
        }
    }
}

/// Trait for scoping strategy implementations.
pub trait ScopingStrategy {
    /// Resolve a name in the given scope.
    ///
    /// # Arguments
    ///
    /// * `graph` - The model graph to search in
    /// * `scope_id` - The starting scope (usually the owning element)
    /// * `name` - The name to resolve
    ///
    /// # Returns
    ///
    /// The resolution result.
    fn resolve(&self, graph: &ModelGraph, scope_id: &ElementId, name: &str) -> ScopedResolution;
}

/// Convenience function to resolve using the appropriate strategy.
pub fn resolve_with_strategy(
    graph: &ModelGraph,
    scope_id: &ElementId,
    name: &str,
    strategy: &crate::crossrefs::ScopeStrategy,
) -> ScopedResolution {
    use crate::crossrefs::ScopeStrategy;

    match strategy {
        ScopeStrategy::OwningNamespace => resolve_in_owning_namespace(graph, scope_id, name),
        ScopeStrategy::NonExpressionNamespace => {
            resolve_in_non_expression_namespace(graph, scope_id, name)
        }
        ScopeStrategy::RelativeNamespace => resolve_in_relative_namespace(graph, scope_id, name),
        ScopeStrategy::FeatureChaining => resolve_with_feature_chaining(graph, scope_id, name),
        ScopeStrategy::TransitionSpecific => resolve_in_transition_context(graph, scope_id, name),
        ScopeStrategy::Global => resolve_in_global_scope(graph, name),
    }
}
