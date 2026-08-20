//! Owning namespace scoping strategy.
//!
//! This is the most common scoping strategy, used for most cross-references.
//! It resolves names by:
//! 1. Looking in the owning namespace's owned memberships
//! 2. Looking in inherited members (for Types)
//! 3. Looking in imported members
//! 4. Walking up to parent namespaces
//! 5. Falling back to global scope (root packages)
//! 6. Checking library packages

use super::ScopedResolution;
use crate::ElementId;
use crate::ModelGraph;

/// Resolve a name in the owning namespace hierarchy.
///
/// This implements the standard SysML v2 scoping rules:
/// 1. OWNED - local owned memberships
/// 2. INHERITED - members via Specialization chain
/// 3. IMPORTED - members from Import statements
/// 4. PARENT - walk up to parent namespace
/// 5. GLOBAL - root packages
/// 6. LIBRARY - standard library packages
pub fn resolve_in_owning_namespace(
    graph: &ModelGraph,
    scope_id: &ElementId,
    name: &str,
) -> ScopedResolution {
    // Use the existing ResolutionContext for full resolution
    let mut ctx = crate::resolution::ResolutionContext::new(graph);

    match ctx.resolve_name(scope_id, name) {
        Some(id) => ScopedResolution::Found(id),
        None => ScopedResolution::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_in_owning_namespace_not_found() {
        let graph = ModelGraph::new();
        let scope = ElementId::new_v4();

        let result = resolve_in_owning_namespace(&graph, &scope, "NonExistent");
        assert!(matches!(result, ScopedResolution::NotFound));
    }
}
