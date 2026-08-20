//! Transition-specific scoping strategy.
//!
//! This strategy is used for resolving references within state machine
//! transitions, particularly for:
//! - Transition sources (the state we transition FROM)
//! - Transition targets (the state we transition TO)
//! - Trigger events
//! - Guard conditions
//! - Effect actions
//!
//! ## Scoping Rules (based on Xtext pilot implementation)
//!
//! | Element | Resolution Namespace |
//! |---------|---------------------|
//! | Source state | Parent namespace of TransitionUsage (e.g., containing StateDefinition) |
//! | Target state | Parent namespace of TransitionUsage (e.g., containing StateDefinition) |
//! | Triggers | TransitionUsage itself, then parent namespace |
//! | Guards | TransitionUsage itself, then parent namespace |
//! | Effects | TransitionUsage itself, then parent namespace |
//!
//! This allows transition sources and targets to reference sibling states
//! within the same state machine.

use super::ScopedResolution;
use crate::resolution::res_trace;
use crate::ElementId;
use crate::ModelGraph;

/// Resolve a name in a transition context.
///
/// For transition sources and targets, this uses the owning namespace
/// (parent of the TransitionUsage), which is typically the containing
/// StateDefinition. This allows referencing sibling states.
///
/// For triggers, guards, and effects, standard element scoping applies
/// (from the TransitionUsage, then parent namespace).
pub fn resolve_in_transition_context(
    graph: &ModelGraph,
    scope_id: &ElementId,
    name: &str,
) -> ScopedResolution {
    res_trace!(
        strategy = "transition_specific",
        query_name = %name,
        scope_id = ?scope_id,
        "resolution strategy selected"
    );

    // Get the element
    let Some(element) = graph.get_element(scope_id) else {
        res_trace!(
            strategy = "transition_specific",
            query_name = %name,
            scope_id = ?scope_id,
            result = "scope_not_found",
            "transition context scope not found"
        );
        return ScopedResolution::NotFound;
    };

    use crate::ElementKind;

    // Check if this is a transition-related element
    let is_transition = matches!(element.kind, ElementKind::TransitionUsage);

    let is_transition_feature = matches!(
        element.kind,
        ElementKind::AcceptActionUsage
            | ElementKind::SendActionUsage
            | ElementKind::TriggerInvocationExpression
            | ElementKind::StateSubactionMembership
            | ElementKind::TransitionFeatureMembership
    );

    if is_transition {
        res_trace!(
            strategy = "transition_specific",
            query_name = %name,
            scope_id = ?scope_id,
            scope_kind = ?element.kind,
            branch = "transition_usage",
            "transition context branch selected"
        );
        // For the transition itself, use owning namespace
        // This allows resolving source and target states from the containing state machine
        super::owning::resolve_in_owning_namespace(graph, scope_id, name)
    } else if is_transition_feature {
        res_trace!(
            strategy = "transition_specific",
            query_name = %name,
            scope_id = ?scope_id,
            scope_kind = ?element.kind,
            branch = "transition_feature",
            "transition context branch selected"
        );
        // For triggers, guards, effects - find the containing transition first
        // Then resolve from there using owning namespace
        if let Some(transition_id) = find_containing_transition(graph, scope_id) {
            res_trace!(
                strategy = "transition_specific",
                query_name = %name,
                scope_id = ?scope_id,
                containing_transition_id = ?transition_id,
                "found containing transition for feature scope"
            );
            super::owning::resolve_in_owning_namespace(graph, &transition_id, name)
        } else {
            res_trace!(
                strategy = "transition_specific",
                query_name = %name,
                scope_id = ?scope_id,
                branch = "transition_feature_fallback",
                "no containing transition found, falling back to current scope"
            );
            // Fallback: resolve from current scope
            super::owning::resolve_in_owning_namespace(graph, scope_id, name)
        }
    } else {
        res_trace!(
            strategy = "transition_specific",
            query_name = %name,
            scope_id = ?scope_id,
            scope_kind = ?element.kind,
            branch = "non_transition_fallback",
            "not in transition context, using owning namespace"
        );
        // Not a transition context, fall back to owning namespace
        super::owning::resolve_in_owning_namespace(graph, scope_id, name)
    }
}

/// Find the containing TransitionUsage for an element.
fn find_containing_transition(graph: &ModelGraph, element_id: &ElementId) -> Option<ElementId> {
    use crate::ElementKind;

    let mut current_id = element_id.clone();

    // Walk up the ownership chain looking for a TransitionUsage
    loop {
        let element = graph.get_element(&current_id)?;

        if matches!(element.kind, ElementKind::TransitionUsage) {
            return Some(current_id);
        }

        // Walk up to owner
        current_id = element.owner.clone()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_transition_not_found() {
        let graph = ModelGraph::new();
        let scope = ElementId::new_v4();

        let result = resolve_in_transition_context(&graph, &scope, "NonExistent");
        assert!(matches!(result, ScopedResolution::NotFound));
    }
}
