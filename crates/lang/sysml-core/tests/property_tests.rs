//! Property-based tests for ModelGraph invariants.

use proptest::prelude::*;
use std::collections::HashSet;
use sysml_core::{Element, ElementId, ElementKind, ModelGraph};

/// Strategy to pick a random ElementKind from a representative set.
fn arb_element_kind() -> impl Strategy<Value = ElementKind> {
    prop_oneof![
        Just(ElementKind::Package),
        Just(ElementKind::PartUsage),
        Just(ElementKind::PartDefinition),
        Just(ElementKind::AttributeUsage),
        Just(ElementKind::RequirementUsage),
        Just(ElementKind::ConstraintUsage),
    ]
}

/// Build a random graph with 5-20 elements and random tree ownership.
///
/// We first add all elements without owners, then randomly assign owners
/// to form a forest (tree-structured ownership, no cycles).
fn arb_graph_with_ownership() -> impl Strategy<Value = ModelGraph> {
    (5usize..=20usize)
        .prop_flat_map(|n| {
            let kinds = prop::collection::vec(arb_element_kind(), n);
            let names = prop::collection::vec(prop::option::of("[a-zA-Z_][a-zA-Z0-9_]{0,10}"), n);
            // For each element after the first, optionally pick a parent
            // from elements with a lower index (ensures acyclic tree).
            let parent_indices =
                prop::collection::vec(prop::option::of(any::<prop::sample::Index>()), n);
            (kinds, names, parent_indices)
        })
        .prop_map(|(kinds, names, parent_indices)| {
            let mut graph = ModelGraph::new();
            let mut ids: Vec<ElementId> = Vec::new();

            // First pass: create all elements without owners
            for (i, kind) in kinds.iter().enumerate() {
                let mut elem = Element::new_with_kind(kind.clone());
                if let Some(Some(name)) = names.get(i) {
                    elem.name = Some(name.clone());
                }
                let id = graph.add_element(elem);
                ids.push(id);
            }

            // Second pass: set owners (only to earlier elements to guarantee acyclicity)
            for i in 1..ids.len() {
                if let Some(Some(idx)) = parent_indices.get(i) {
                    // Pick a parent from elements 0..i
                    let parent_pos = idx.index(i);
                    let parent_id = ids[parent_pos].clone();
                    let child_id = ids[i].clone();
                    if let Some(elem) = graph.get_element_mut(&child_id) {
                        elem.owner = Some(parent_id);
                    }
                }
            }

            // Rebuild indexes to reflect the owner mutations
            graph.rebuild_indexes();
            graph
        })
}

proptest! {
    /// Owner chains must be acyclic: following .owner pointers never revisits an element.
    #[test]
    fn owner_chain_acyclicity(graph in arb_graph_with_ownership()) {
        for (id, element) in &graph.elements {
            let mut visited = HashSet::new();
            visited.insert(id.clone());

            let mut current_owner = element.owner.clone();
            while let Some(owner_id) = current_owner {
                prop_assert!(
                    visited.insert(owner_id.clone()),
                    "Cycle detected in owner chain for element {}",
                    id
                );
                current_owner = graph
                    .get_element(&owner_id)
                    .and_then(|e| e.owner.clone());
            }
        }
    }

    /// root_ids must be exactly the set of elements with owner == None.
    #[test]
    fn root_ids_consistency(graph in arb_graph_with_ownership()) {
        let roots_from_method: HashSet<ElementId> = graph
            .roots()
            .map(|e| e.id.clone())
            .collect();

        let roots_from_scan: HashSet<ElementId> = graph
            .elements
            .values()
            .filter(|e| e.owner.is_none())
            .map(|e| e.id.clone())
            .collect();

        prop_assert_eq!(roots_from_method, roots_from_scan);
    }

    /// Adding an element is idempotent: re-adding the same element (same ID)
    /// does not increase element_count.
    #[test]
    fn add_idempotence(kind in arb_element_kind(), name in "[a-zA-Z_]{1,8}") {
        let mut graph = ModelGraph::new();

        let elem = Element::new_with_kind(kind).with_name(name);
        let id = elem.id.clone();
        let elem_clone = elem.clone();

        graph.add_element(elem);
        prop_assert_eq!(graph.element_count(), 1);

        // Re-add the same element (same ID)
        graph.add_element(elem_clone);
        // BTreeMap::insert replaces, so count stays 1
        prop_assert_eq!(graph.element_count(), 1);

        // The element is still retrievable
        prop_assert!(graph.get_element(&id).is_some());
    }

    /// Every element referenced as a child via children_of() must have that parent as its owner.
    #[test]
    fn membership_consistency(graph in arb_graph_with_ownership()) {
        for (parent_id, _) in &graph.elements {
            for child in graph.children_of(parent_id) {
                prop_assert_eq!(
                    child.owner.as_ref(),
                    Some(parent_id),
                    "Child {} claims owner {:?} but was found via children_of({})",
                    child.id, child.owner, parent_id
                );
            }
        }
    }
}
