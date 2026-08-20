//! Namespace operations for SysML v2 model graphs.
//!
//! In SysML v2, a Namespace is a Type that contains members through Memberships.
//! This module provides operations for:
//! - Getting owned memberships and members of a namespace
//! - Resolving names within a namespace
//! - Resolving qualified names from the root
//!
//! ## Visibility
//!
//! Members have visibility (public/private/protected):
//! - Public members are visible outside the namespace
//! - Private members are only visible within the namespace
//! - Protected members are visible to specializations

use crate::membership::{props as membership_props, MembershipView};
use crate::{Element, ElementKind, ModelGraph, VisibilityKind};
use sysml_id::ElementId;

impl ModelGraph {
    /// Get the owned memberships of a namespace.
    ///
    /// Returns all OwningMembership elements where this namespace is the
    /// `membershipOwningNamespace`.
    pub fn owned_memberships(&self, namespace_id: &ElementId) -> impl Iterator<Item = &Element> {
        // Use the index if available
        let membership_ids = self.namespace_to_memberships.get(namespace_id);

        membership_ids
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(move |id| self.elements.get(id))
            // Filter to only OwningMembership types
            .filter(|e| {
                e.kind == ElementKind::OwningMembership
                    || e.kind.is_subtype_of(ElementKind::OwningMembership)
            })
    }

    /// Get all memberships of a namespace (including non-owning).
    ///
    /// Returns all Membership elements where this namespace is the
    /// `membershipOwningNamespace`.
    pub fn memberships(&self, namespace_id: &ElementId) -> impl Iterator<Item = &Element> {
        let membership_ids = self.namespace_to_memberships.get(namespace_id);

        membership_ids
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(move |id| self.elements.get(id))
    }

    /// Get the owned members (elements) of a namespace.
    ///
    /// Returns the elements that are owned by this namespace through OwningMemberships.
    /// This is the collection of elements for which this namespace is the owner.
    pub fn owned_members(&self, namespace_id: &ElementId) -> impl Iterator<Item = &Element> {
        self.owned_memberships(namespace_id)
            .filter_map(|membership| {
                membership
                    .props
                    .get(membership_props::MEMBER_ELEMENT)
                    .and_then(|v| v.as_ref())
            })
            .filter_map(move |member_id| self.elements.get(member_id))
    }

    /// Get all members (elements) of a namespace.
    ///
    /// Returns all elements that are members of this namespace through any Membership.
    pub fn members(&self, namespace_id: &ElementId) -> impl Iterator<Item = &Element> {
        self.memberships(namespace_id)
            .filter_map(|membership| {
                membership
                    .props
                    .get(membership_props::MEMBER_ELEMENT)
                    .and_then(|v| v.as_ref())
            })
            .filter_map(move |member_id| self.elements.get(member_id))
    }

    /// Get the visible (public) members of a namespace.
    ///
    /// Returns members whose membership has `visibility = public`.
    pub fn visible_members(&self, namespace_id: &ElementId) -> impl Iterator<Item = &Element> {
        self.memberships(namespace_id)
            .filter(|membership| {
                MembershipView::try_from_element(membership)
                    .map(|v| v.is_public())
                    .unwrap_or(false)
            })
            .filter_map(|membership| {
                membership
                    .props
                    .get(membership_props::MEMBER_ELEMENT)
                    .and_then(|v| v.as_ref())
            })
            .filter_map(move |member_id| self.elements.get(member_id))
    }

    /// Get members with a specific visibility.
    pub fn members_with_visibility(
        &self,
        namespace_id: &ElementId,
        visibility: VisibilityKind,
    ) -> impl Iterator<Item = &Element> {
        self.memberships(namespace_id)
            .filter(move |membership| {
                MembershipView::try_from_element(membership)
                    .map(|v| v.visibility() == visibility)
                    .unwrap_or(false)
            })
            .filter_map(|membership| {
                membership
                    .props
                    .get(membership_props::MEMBER_ELEMENT)
                    .and_then(|v| v.as_ref())
            })
            .filter_map(move |member_id| self.elements.get(member_id))
    }

    /// Resolve a name within a namespace.
    ///
    /// Looks for a member whose `memberName` (in the Membership) matches the given name.
    /// Falls back to checking the member element's `name` if `memberName` is not set.
    ///
    /// # Returns
    ///
    /// The member element with the matching name, or None if not found.
    pub fn resolve_name(&self, namespace_id: &ElementId, name: &str) -> Option<&Element> {
        for membership in self.memberships(namespace_id) {
            // Check memberName in the membership first
            let member_name = membership
                .props
                .get(membership_props::MEMBER_NAME)
                .and_then(|v| v.as_str());

            // Get the member element ID
            let member_id = membership
                .props
                .get(membership_props::MEMBER_ELEMENT)
                .and_then(|v| v.as_ref())?;

            let member = self.elements.get(member_id)?;

            // Match against memberName or element's name
            let matches = member_name
                .map(|mn| mn == name)
                .or_else(|| member.name.as_ref().map(|n| n == name))
                .unwrap_or(false);

            if matches {
                return Some(member);
            }
        }
        None
    }

    /// Resolve a qualified name path from the roots.
    ///
    /// The qualified name should be in the format `Segment1::Segment2::...::SegmentN`.
    /// Resolution starts from the root elements and follows the path.
    ///
    /// # Example
    ///
    /// ```ignore
    /// graph.resolve_qname("MyPackage::SubPackage::PartDef")
    /// ```
    ///
    /// # Returns
    ///
    /// The element at the end of the path, or None if resolution fails.
    #[allow(clippy::indexing_slicing)] // segments[0] safe: checked !is_empty() above
    pub fn resolve_qname(&self, qname: &str) -> Option<&Element> {
        let segments: Vec<&str> = qname.split("::").collect();
        if segments.is_empty() {
            return None;
        }

        // Find the root element matching the first segment.
        // Ridge B: indexed lookup instead of linear `roots().find()`.
        let first_name = segments[0];
        let mut current = self.lookup_root_by_name(first_name)?;

        // Resolve each subsequent segment
        for segment in segments.iter().skip(1) {
            current = self.resolve_name(&current.id, segment)?;
        }

        Some(current)
    }

    /// Resolve a path relative to a namespace.
    ///
    /// Similar to `resolve_qname` but starts from a specific namespace instead of root.
    pub fn resolve_path(&self, namespace_id: &ElementId, path: &str) -> Option<&Element> {
        let segments: Vec<&str> = path.split("::").collect();
        if segments.is_empty() {
            return None;
        }

        let mut current_id = namespace_id.clone();
        for segment in segments {
            let member = self.resolve_name(&current_id, segment)?;
            current_id = member.id.clone();
        }

        self.elements.get(&current_id)
    }

    /// Get all descendants of a namespace (recursively owned elements).
    ///
    /// Returns all elements owned directly or indirectly by this namespace.
    pub fn descendants(&self, namespace_id: &ElementId) -> Vec<&Element> {
        let mut result = Vec::new();
        let mut stack: Vec<&ElementId> = vec![namespace_id];

        while let Some(current_id) = stack.pop() {
            for member in self.owned_members(current_id) {
                result.push(member);
                stack.push(&member.id);
            }
        }

        result
    }

    /// Check if an element is a descendant of a namespace.
    pub fn is_descendant_of(&self, element_id: &ElementId, namespace_id: &ElementId) -> bool {
        for ancestor in self.ancestors(element_id) {
            if ancestor.id == *namespace_id {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Element, ElementKind, VisibilityKind};

    fn create_test_hierarchy() -> (ModelGraph, ElementId, ElementId, ElementId) {
        let mut graph = ModelGraph::new();

        // Create TestPackage::SubPackage::PartDef
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPackage");
        let pkg_id = graph.add_element(pkg);

        let sub = Element::new_with_kind(ElementKind::Package).with_name("SubPackage");
        let sub_id = graph.add_owned_element(sub, pkg_id.clone(), VisibilityKind::Public);

        let part = Element::new_with_kind(ElementKind::PartDefinition).with_name("PartDef");
        let part_id = graph.add_owned_element(part, sub_id.clone(), VisibilityKind::Public);

        (graph, pkg_id, sub_id, part_id)
    }

    #[test]
    fn owned_members_basic() {
        let (graph, pkg_id, sub_id, _) = create_test_hierarchy();

        let members: Vec<_> = graph.owned_members(&pkg_id).collect();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].id, sub_id);
    }

    #[test]
    fn resolve_name_by_member_name() {
        let (graph, pkg_id, sub_id, _) = create_test_hierarchy();

        let resolved = graph.resolve_name(&pkg_id, "SubPackage").unwrap();
        assert_eq!(resolved.id, sub_id);
    }

    #[test]
    fn resolve_name_not_found() {
        let (graph, pkg_id, _, _) = create_test_hierarchy();

        let resolved = graph.resolve_name(&pkg_id, "NonExistent");
        assert!(resolved.is_none());
    }

    #[test]
    fn resolve_qname_full_path() {
        let (graph, _, _, part_id) = create_test_hierarchy();

        let resolved = graph
            .resolve_qname("TestPackage::SubPackage::PartDef")
            .unwrap();
        assert_eq!(resolved.id, part_id);
    }

    #[test]
    fn resolve_qname_partial_path() {
        let (graph, _, sub_id, _) = create_test_hierarchy();

        let resolved = graph.resolve_qname("TestPackage::SubPackage").unwrap();
        assert_eq!(resolved.id, sub_id);
    }

    #[test]
    fn resolve_qname_root_only() {
        let (graph, pkg_id, _, _) = create_test_hierarchy();

        let resolved = graph.resolve_qname("TestPackage").unwrap();
        assert_eq!(resolved.id, pkg_id);
    }

    #[test]
    fn resolve_qname_not_found() {
        let (graph, _, _, _) = create_test_hierarchy();

        assert!(graph.resolve_qname("NonExistent::Path").is_none());
        assert!(graph.resolve_qname("TestPackage::NonExistent").is_none());
    }

    #[test]
    fn visible_members_filters_private() {
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        // Add a public member
        let public_part =
            Element::new_with_kind(ElementKind::PartDefinition).with_name("PublicPart");
        let _public_id =
            graph.add_owned_element(public_part, pkg_id.clone(), VisibilityKind::Public);

        // Add a private member
        let private_part =
            Element::new_with_kind(ElementKind::PartDefinition).with_name("PrivatePart");
        let _private_id =
            graph.add_owned_element(private_part, pkg_id.clone(), VisibilityKind::Private);

        // owned_members returns both
        let all: Vec<_> = graph.owned_members(&pkg_id).collect();
        assert_eq!(all.len(), 2);

        // visible_members returns only public
        let visible: Vec<_> = graph.visible_members(&pkg_id).collect();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, Some("PublicPart".to_string()));
    }

    #[test]
    fn descendants_recursive() {
        let (graph, pkg_id, _, _) = create_test_hierarchy();

        let desc = graph.descendants(&pkg_id);
        assert_eq!(desc.len(), 2); // SubPackage and PartDef
    }

    #[test]
    fn is_descendant_of_check() {
        let (graph, pkg_id, _, part_id) = create_test_hierarchy();

        assert!(graph.is_descendant_of(&part_id, &pkg_id));
        assert!(!graph.is_descendant_of(&pkg_id, &part_id));
    }

    #[test]
    fn resolve_path_from_namespace() {
        let (graph, pkg_id, _, part_id) = create_test_hierarchy();

        let resolved = graph.resolve_path(&pkg_id, "SubPackage::PartDef").unwrap();
        assert_eq!(resolved.id, part_id);
    }
}
