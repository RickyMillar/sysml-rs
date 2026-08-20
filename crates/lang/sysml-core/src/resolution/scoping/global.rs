//! Global scoping strategy.
//!
//! This strategy resolves names in the global scope, which includes:
//! - Root packages (top-level packages without owners)
//! - Library packages (registered standard library packages)
//!
//! This is used for imports and other references that need to access
//! top-level elements.

use super::ScopedResolution;
use crate::ElementId;
use crate::ModelGraph;

/// Resolve a name in the global scope.
///
/// Global scope includes:
/// 1. Root packages (packages without owners)
/// 2. Registered library packages
pub fn resolve_in_global_scope(graph: &ModelGraph, name: &str) -> ScopedResolution {
    // Look in root packages first
    if let Some(id) = resolve_in_root_packages(graph, name) {
        return ScopedResolution::Found(id);
    }

    // Then look in library packages
    if let Some(id) = resolve_in_library_packages(graph, name) {
        return ScopedResolution::Found(id);
    }

    ScopedResolution::NotFound
}

/// Resolve a name in root packages.
///
/// Iterates `graph.roots()` (insertion-ordered via `root_ids`, already filtered
/// to `owner.is_none()`) rather than the unordered `graph.elements` FxHashMap.
/// This makes global-tier resolution deterministic across runs when multiple
/// root packages share a name (ADR-016 D5).
fn resolve_in_root_packages(graph: &ModelGraph, name: &str) -> Option<ElementId> {
    use crate::ElementKind;

    // Find all root packages (packages without owners), in stable insertion order.
    for element in graph.roots() {
        if element.kind == ElementKind::Package {
            // Check if this package's name matches
            if element.name.as_deref() == Some(name) {
                return Some(element.id.clone());
            }

            // Also check members of the root package
            if let Some(member_id) = resolve_member_by_name(graph, &element.id, name) {
                return Some(member_id);
            }
        }
    }

    None
}

/// Resolve a name in library packages.
///
/// Uses a two-tier strategy:
/// 1. **Fast path**: O(1) FxHashMap lookup via `graph.resolve_in_library()`
/// 2. **Fallback**: Recursive scan only if the fast path misses
fn resolve_in_library_packages(graph: &ModelGraph, name: &str) -> Option<ElementId> {
    // Fast path: O(1) lookup in pre-built library name index
    if let Some(id) = graph.resolve_in_library(name) {
        return Some(id.clone());
    }

    // Fallback: recursive scan for names the index may have missed
    for lib_id in graph.library_packages() {
        // Check if the library package itself matches
        if let Some(lib) = graph.get_element(lib_id) {
            if lib.name.as_deref() == Some(name) {
                return Some(lib_id.clone());
            }
        }

        // Search recursively in the library package
        if let Some(member_id) = search_library_recursively(graph, lib_id, name, 0) {
            return Some(member_id);
        }
    }

    None
}

/// Check if two names match, stripping quotes if present.
fn names_match(name: &str, target: &str) -> bool {
    let name = name.trim_matches('\'');
    let target = target.trim_matches('\'');
    name == target
}

/// Recursively search a library namespace for a name.
fn search_library_recursively(
    graph: &ModelGraph,
    namespace_id: &ElementId,
    name: &str,
    depth: usize,
) -> Option<ElementId> {
    use crate::ElementKind;

    const MAX_DEPTH: usize = 10;
    if depth > MAX_DEPTH {
        return None;
    }

    // Check direct members first
    for member in graph.owned_members(namespace_id) {
        if let Some(mname) = member.name.as_deref() {
            if names_match(mname, name) {
                return Some(member.id.clone());
            }
        }

        // Recurse into nested namespaces
        if member.kind.is_subtype_of(ElementKind::Namespace)
            || member.kind == ElementKind::Namespace
            || member.kind == ElementKind::Package
        {
            if let Some(id) = search_library_recursively(graph, &member.id, name, depth + 1) {
                return Some(id);
            }
        }
    }

    // Also follow public namespace imports to find re-exported types
    for member in graph.owned_members(namespace_id) {
        let is_import = member.kind == ElementKind::Import
            || member.kind == ElementKind::NamespaceImport
            || member.kind == ElementKind::MembershipImport
            || member.kind.is_subtype_of(ElementKind::Import);

        if !is_import {
            continue;
        }

        // Check visibility - only follow public imports
        let is_public = member
            .props
            .get("visibility")
            .and_then(|v| v.as_str())
            .map(|v| v == "public")
            .unwrap_or(false);

        if !is_public {
            continue;
        }

        // Check if it's a namespace import (::*)
        let is_namespace = member
            .props
            .get("isNamespace")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !is_namespace {
            continue;
        }

        // Get the imported reference - need to resolve it
        // For simplicity in the global scope, we skip complex resolution
        // The main resolution context handles this more thoroughly
    }

    None
}

/// Resolve a member by name within a namespace.
fn resolve_member_by_name(
    graph: &ModelGraph,
    namespace_id: &ElementId,
    name: &str,
) -> Option<ElementId> {
    for member in graph.owned_members(namespace_id) {
        if member.name.as_deref() == Some(name) {
            return Some(member.id.clone());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_global_not_found() {
        let graph = ModelGraph::new();

        let result = resolve_in_global_scope(&graph, "NonExistent");
        assert!(matches!(result, ScopedResolution::NotFound));
    }
}
