//! Non-expression namespace scoping strategy.
//!
//! This strategy is used for cross-references that should skip expression scopes
//! when walking up the namespace hierarchy. It's used for:
//! - FeatureTyping
//! - Conjugation
//! - Other type-related references
//!
//! The key difference from OwningNamespace is that expression scopes are skipped.
//!
//! ## Algorithm (based on Xtext pilot implementation)
//!
//! The algorithm walks up through these element types:
//! - `FeatureValue` - a membership providing a feature's value
//! - `InstantiationExpression` - expression instantiating a type
//! - `FeatureReferenceExpression` - expression referencing a feature
//!
//! Until it finds a stable declaration namespace.

use super::ScopedResolution;
use crate::resolution::res_trace;
use crate::ElementId;
use crate::ModelGraph;

/// Resolve a name, skipping expression scopes.
///
/// This implements the SysML v2 scoping rule for FeatureTyping and similar
/// cross-references. Expression contexts (FeatureValue, InstantiationExpression,
/// FeatureReferenceExpression) are skipped to find a stable declaration namespace.
pub fn resolve_in_non_expression_namespace(
    graph: &ModelGraph,
    scope_id: &ElementId,
    name: &str,
) -> ScopedResolution {
    res_trace!(
        strategy = "non_expression_namespace",
        query_name = %name,
        scope_id = ?scope_id,
        "resolution strategy selected"
    );

    // Find the non-expression namespace for this scope
    let effective_scope = find_non_expression_namespace(graph, scope_id);

    #[cfg(feature = "resolution-tracing")]
    if &effective_scope != scope_id {
        let orig_name = graph
            .get_element(scope_id)
            .and_then(|e| e.name.clone())
            .unwrap_or_else(|| format!("{:.8}", scope_id));
        let eff_name = graph
            .get_element(&effective_scope)
            .and_then(|e| e.name.clone())
            .unwrap_or_else(|| format!("{:.8}", effective_scope));
        res_trace!(
            strategy = "non_expression_namespace",
            query_name = %name,
            original_scope_id = ?scope_id,
            effective_scope_id = ?effective_scope,
            original_scope_name = %orig_name,
            effective_scope_name = %eff_name,
            "expression scopes skipped before lookup"
        );
    }

    super::owning::resolve_in_owning_namespace(graph, &effective_scope, name)
}

/// Find the nearest non-expression namespace for a given element.
///
/// Based on `NamespaceUtil.getNonExpressionNamespaceFor()` from the Xtext pilot:
/// - Walks up through FeatureValue memberships
/// - Skips InstantiationExpression and FeatureReferenceExpression namespaces
/// - Returns the first stable declaration namespace
pub(crate) fn find_non_expression_namespace(graph: &ModelGraph, element_id: &ElementId) -> ElementId {
    use crate::ElementKind;

    let mut current_id = element_id.clone();

    // Keep walking up until we find a non-expression namespace
    loop {
        let Some(element) = graph.get_element(&current_id) else {
            return current_id;
        };

        // Check if the owning membership is a FeatureValue
        let is_in_feature_value = element
            .owner
            .as_ref()
            .and_then(|owner_id| graph.get_element(owner_id))
            .map(|owner| matches!(owner.kind, ElementKind::FeatureValue))
            .unwrap_or(false);

        // Check if this element is an expression type that should be skipped
        // Based on the Xtext pilot, we specifically check for:
        // - InstantiationExpression
        // - FeatureReferenceExpression
        // We also include other expression types for completeness
        let is_expression_namespace = matches!(
            element.kind,
            ElementKind::ConstructorExpression
                | ElementKind::FeatureReferenceExpression
                | ElementKind::InvocationExpression
                | ElementKind::OperatorExpression
                | ElementKind::FeatureChainExpression
                | ElementKind::SelectExpression
                | ElementKind::CollectExpression
                | ElementKind::IndexExpression
                | ElementKind::MetadataAccessExpression
                | ElementKind::NullExpression
                | ElementKind::LiteralExpression
                | ElementKind::LiteralBoolean
                | ElementKind::LiteralInteger
                | ElementKind::LiteralRational
                | ElementKind::LiteralInfinity
                | ElementKind::LiteralString
        );

        if is_in_feature_value || is_expression_namespace {
            // Walk up to the owner
            if let Some(owner_id) = &element.owner {
                res_trace!(
                    strategy = "non_expression_namespace",
                    skipped_kind = ?element.kind,
                    skipped_name = element.name.as_deref().unwrap_or("unnamed"),
                    next_scope_id = ?owner_id,
                    "skipping expression scope while resolving"
                );
                current_id = owner_id.clone();
                continue;
            }
        }

        // Found a non-expression namespace
        return current_id;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_non_expression_not_found() {
        let graph = ModelGraph::new();
        let scope = ElementId::new_v4();

        let result = resolve_in_non_expression_namespace(&graph, &scope, "NonExistent");
        assert!(matches!(result, ScopedResolution::NotFound));
    }
}
