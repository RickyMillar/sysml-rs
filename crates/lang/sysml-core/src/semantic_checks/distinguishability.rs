//! Namespace distinguishability checks.
//!
//! Validates that member names within a namespace are unique.

use crate::validation::{SemanticError, SemanticErrorKind};
use crate::{Element, ElementKind, ModelGraph};
use rustc_hash::FxHashMap;

/// Check that all owned members of a namespace have unique names.
///
/// Rule S001: Duplicate owned member name.
pub fn unique_owned_member_names(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    // Only applies to namespace-like elements
    if !element.kind.is_subtype_of(ElementKind::Namespace)
        && element.kind != ElementKind::Namespace
        && element.kind != ElementKind::Package
    {
        return None;
    }

    let mut errors = Vec::new();
    let mut seen_names: FxHashMap<String, sysml_id::ElementId> = FxHashMap::default();

    // Sort children by source position so S001 consistently flags the
    // second occurrence in the source, not whichever comes first in
    // HashMap iteration order.
    let mut children: Vec<_> = graph.children_of(&element.id).collect();
    children.sort_by_key(|c| c.spans.first().map_or(usize::MAX, |s| s.start));

    // Check all direct children for name collisions
    for child in &children {
        // Skip satisfy usages — their "name" is actually the referenced
        // requirement, not a declared member name, so collisions are expected.
        // Skip succession-as-usage — `then X;` creates a SuccessionAsUsage
        // named "X" that would collide with the actual `action X { }`.
        if matches!(
            child.kind,
            ElementKind::SatisfyRequirementUsage | ElementKind::SuccessionAsUsage
        ) {
            continue;
        }
        // Skip expression-tree nodes — the parser names reference nodes after
        // their referent path (e.g. two FeatureReferenceExpressions both named
        // "s1.x" under one OperatorExpression in `s1.x * s1.x`). Per spec these
        // are unnamed; references are not declared member names, so they carry
        // no distinguishability obligation.
        if child.kind == ElementKind::Expression
            || child.kind.is_subtype_of(ElementKind::Expression)
        {
            continue;
        }

        if let Some(name) = &child.name {
            if name.is_empty() {
                continue;
            }
            if let Some(existing_id) = seen_names.get(name) {
                errors.push(SemanticError {
                    element_id: child.id.clone(),
                    element_name: Some(name.clone()),
                    kind: SemanticErrorKind::DuplicateName {
                        name: name.clone(),
                        other_id: existing_id.clone(),
                    },
                    rule_id: "S001",
                    is_warning: true,
                });
            } else {
                seen_names.insert(name.clone(), child.id.clone());
            }
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Check that member names don't conflict with aliases.
///
/// Rule S002: Member name duplicates an alias.
pub fn no_name_alias_conflict(
    _element: &Element,
    _graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    // Alias support is not yet implemented in the model.
    // This is a placeholder for future implementation.
    None
}

/// Check that member names don't conflict with inherited member names.
///
/// Rule S003: Member name conflicts with inherited member.
///
/// FILED GAP (task #85 steward audit, 2026-08-12): this check is a
/// placeholder, so owned-member vs inherited-member name collisions —
/// including a member of a feature's resolved TYPE colliding with one of the
/// feature's own owned members — are never flagged. KerML makes this a
/// well-formedness constraint, not a precedence rule ("The member names of
/// all inherited memberships must be distinct from each other and from the
/// member names of all owned memberships", KerML-spec-r2025-04.txt:2523-2537,
/// with the note that redefinition is the sanctioned way to resolve such
/// conflicts — so an implementation must EXEMPT redefining members). Until
/// this lands, `scoping::resolve_with_feature_chaining`'s owned-members-first
/// traversal silently picks the owned member in an ill-formed model instead
/// of the model failing validation.
pub fn no_inherited_name_conflict(
    _element: &Element,
    _graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    // Inherited member tracking is not yet fully implemented.
    // This is a placeholder for future implementation.
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_id::ElementId;

    #[test]
    fn no_duplicates_passes() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new(ElementId::new_v4(), ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let child1 = Element::new(ElementId::new_v4(), ElementKind::PartUsage)
            .with_name("a")
            .with_owner(pkg_id.clone());
        graph.add_element(child1);

        let child2 = Element::new(ElementId::new_v4(), ElementKind::PartUsage)
            .with_name("b")
            .with_owner(pkg_id.clone());
        graph.add_element(child2);

        let pkg_elem = graph.get_element(&pkg_id).unwrap();
        let result = unique_owned_member_names(pkg_elem, &graph);
        assert!(result.is_none());
    }

    #[test]
    fn duplicates_detected() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new(ElementId::new_v4(), ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let child1 = Element::new(ElementId::new_v4(), ElementKind::PartUsage)
            .with_name("duplicate")
            .with_owner(pkg_id.clone());
        graph.add_element(child1);

        let child2 = Element::new(ElementId::new_v4(), ElementKind::PartUsage)
            .with_name("duplicate")
            .with_owner(pkg_id.clone());
        graph.add_element(child2);

        let pkg_elem = graph.get_element(&pkg_id).unwrap();
        let result = unique_owned_member_names(pkg_elem, &graph);
        assert!(result.is_some());
        let errors = result.unwrap();
        assert_eq!(errors.len(), 1);
        assert!(matches!(
            errors[0].kind,
            SemanticErrorKind::DuplicateName { .. }
        ));
        assert_eq!(errors[0].rule_id, "S001");
        assert!(errors[0].is_warning);
    }

    #[test]
    fn expression_reference_children_ignored() {
        // `s1.x * s1.x` mints two FeatureReferenceExpressions both named
        // "s1.x" under one OperatorExpression (a Namespace subtype). These
        // are references, not declared members — no S001.
        let mut graph = ModelGraph::new();
        let op = Element::new(ElementId::new_v4(), ElementKind::OperatorExpression)
            .with_prop("operator", "*");
        let op_id = graph.add_element(op);

        let ref1 = Element::new(ElementId::new_v4(), ElementKind::FeatureReferenceExpression)
            .with_name("s1.x")
            .with_owner(op_id.clone());
        graph.add_element(ref1);

        let ref2 = Element::new(ElementId::new_v4(), ElementKind::FeatureReferenceExpression)
            .with_name("s1.x")
            .with_owner(op_id.clone());
        graph.add_element(ref2);

        let op_elem = graph.get_element(&op_id).unwrap();
        let result = unique_owned_member_names(op_elem, &graph);
        assert!(
            result.is_none(),
            "expression references must not trip S001: {result:?}"
        );
    }

    #[test]
    fn unnamed_children_ignored() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new(ElementId::new_v4(), ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        // Two unnamed children should not conflict
        let child1 =
            Element::new(ElementId::new_v4(), ElementKind::PartUsage).with_owner(pkg_id.clone());
        graph.add_element(child1);

        let child2 =
            Element::new(ElementId::new_v4(), ElementKind::PartUsage).with_owner(pkg_id.clone());
        graph.add_element(child2);

        let pkg_elem = graph.get_element(&pkg_id).unwrap();
        let result = unique_owned_member_names(pkg_elem, &graph);
        assert!(result.is_none());
    }
}
