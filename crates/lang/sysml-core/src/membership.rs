//! Membership view types for SysML v2 compliant ownership.
//!
//! In SysML v2, ownership is established through Membership elements:
//! - `Membership` relates a member element to a namespace
//! - `OwningMembership` is a Membership that also owns the member element
//!
//! This module provides view types for working with Membership elements
//! that are stored as regular Elements in the graph.
//!
//! ## Key Properties
//!
//! From the spec (KerML-shapes.ttl):
//! - `Membership.memberElement` (Exactly-one): The element that is a member
//! - `Membership.membershipOwningNamespace` (Exactly-one): The namespace that owns this membership
//! - `Membership.visibility` (Exactly-one): public/private/protected
//! - `Membership.memberName` (Zero-or-one): Name of the member in the namespace
//! - `Membership.memberShortName` (Zero-or-one): Short name alias
//!
//! `OwningMembership` adds:
//! - `OwningMembership.ownedMemberElement` (Exactly-one): The element owned by this membership

use std::borrow::Cow;

use crate::{Element, ElementKind, Value, VisibilityKind};
use sysml_id::{CanonicalKey, ElementId};

/// Property keys for Membership elements (matching spec property names).
pub mod props {
    /// The element that is a member of the namespace.
    pub const MEMBER_ELEMENT: &str = "memberElement";
    /// The namespace that owns this membership.
    pub const MEMBERSHIP_OWNING_NAMESPACE: &str = "membershipOwningNamespace";
    /// The visibility of the member (public/private/protected).
    pub const VISIBILITY: &str = "visibility";
    /// The name of the member in the owning namespace.
    pub const MEMBER_NAME: &str = "memberName";
    /// The short name alias of the member.
    pub const MEMBER_SHORT_NAME: &str = "memberShortName";
    /// For OwningMembership: the owned element (same as memberElement but emphasizes ownership).
    pub const OWNED_MEMBER_ELEMENT: &str = "ownedMemberElement";
}

/// A view into an Element that is a Membership.
///
/// Membership is a Relationship between a Namespace (the membershipOwningNamespace)
/// and an Element (the memberElement) that indicates the Element is a member of
/// the Namespace.
#[derive(Debug, Clone, Copy)]
pub struct MembershipView<'a> {
    element: &'a Element,
}

impl<'a> MembershipView<'a> {
    /// Try to create a MembershipView from an Element.
    ///
    /// Returns `Some` if the element kind is Membership or any subtype.
    pub fn try_from_element(element: &'a Element) -> Option<Self> {
        if element.kind.is_subtype_of(ElementKind::Membership)
            || element.kind == ElementKind::Membership
        {
            Some(MembershipView { element })
        } else {
            None
        }
    }

    /// Get the underlying element.
    pub fn element(&self) -> &'a Element {
        self.element
    }

    /// Get the member element ID.
    ///
    /// This is the element that is a member of the namespace.
    pub fn member_element(&self) -> Option<&ElementId> {
        self.element.props.get(props::MEMBER_ELEMENT)?.as_ref()
    }

    /// Get the membership owning namespace ID.
    ///
    /// This is the namespace that contains this membership (and thus the member).
    pub fn membership_owning_namespace(&self) -> Option<&ElementId> {
        self.element
            .props
            .get(props::MEMBERSHIP_OWNING_NAMESPACE)?
            .as_ref()
    }

    /// Get the visibility of the member.
    ///
    /// Defaults to `Public` if not specified.
    pub fn visibility(&self) -> VisibilityKind {
        self.element
            .props
            .get(props::VISIBILITY)
            .and_then(|v| v.as_str())
            .and_then(VisibilityKind::from_str)
            .unwrap_or(VisibilityKind::Public)
    }

    /// Get the member name (the name by which the member is known in the namespace).
    pub fn member_name(&self) -> Option<&str> {
        self.element.props.get(props::MEMBER_NAME)?.as_str()
    }

    /// Get the member short name.
    pub fn member_short_name(&self) -> Option<&str> {
        self.element.props.get(props::MEMBER_SHORT_NAME)?.as_str()
    }

    /// Check if this member is public.
    pub fn is_public(&self) -> bool {
        self.visibility() == VisibilityKind::Public
    }

    /// Check if this member is private.
    pub fn is_private(&self) -> bool {
        self.visibility() == VisibilityKind::Private
    }

    /// Check if this member is protected.
    pub fn is_protected(&self) -> bool {
        self.visibility() == VisibilityKind::Protected
    }
}

/// A view into an Element that is an OwningMembership.
///
/// OwningMembership is a Membership that owns its memberElement, establishing
/// a containment relationship. The ownedMemberElement is automatically the
/// memberElement, but the property emphasizes that ownership is being established.
#[derive(Debug, Clone, Copy)]
pub struct OwningMembershipView<'a> {
    element: &'a Element,
}

impl<'a> OwningMembershipView<'a> {
    /// Try to create an OwningMembershipView from an Element.
    ///
    /// Returns `Some` if the element kind is OwningMembership or any subtype.
    pub fn try_from_element(element: &'a Element) -> Option<Self> {
        if element.kind.is_subtype_of(ElementKind::OwningMembership)
            || element.kind == ElementKind::OwningMembership
        {
            Some(OwningMembershipView { element })
        } else {
            None
        }
    }

    /// Get the underlying element.
    pub fn element(&self) -> &'a Element {
        self.element
    }

    /// Get this as a MembershipView (OwningMembership is a subtype of Membership).
    pub fn as_membership(&self) -> MembershipView<'a> {
        // Safe because OwningMembership is always a Membership
        MembershipView {
            element: self.element,
        }
    }

    /// Get the owned member element ID.
    ///
    /// This is the element that is owned by this membership.
    /// For OwningMembership, this is the same as memberElement.
    pub fn owned_member_element(&self) -> Option<&ElementId> {
        // First try the specific ownedMemberElement property
        self.element
            .props
            .get(props::OWNED_MEMBER_ELEMENT)
            .and_then(|v| v.as_ref())
            // Fall back to memberElement (they should be the same)
            .or_else(|| self.element.props.get(props::MEMBER_ELEMENT)?.as_ref())
    }

    /// Get the membership owning namespace ID.
    ///
    /// This is the namespace that owns this membership (and thus owns the member).
    pub fn membership_owning_namespace(&self) -> Option<&ElementId> {
        self.element
            .props
            .get(props::MEMBERSHIP_OWNING_NAMESPACE)?
            .as_ref()
    }

    /// Get the visibility of the owned member.
    pub fn visibility(&self) -> VisibilityKind {
        self.element
            .props
            .get(props::VISIBILITY)
            .and_then(|v| v.as_str())
            .and_then(VisibilityKind::from_str)
            .unwrap_or(VisibilityKind::Public)
    }

    /// Get the member name.
    pub fn member_name(&self) -> Option<&str> {
        self.element.props.get(props::MEMBER_NAME)?.as_str()
    }
}

/// Builder for creating Membership elements.
#[derive(Debug, Clone)]
pub struct MembershipBuilder {
    kind: ElementKind,
    member_element: Option<ElementId>,
    owning_namespace: Option<ElementId>,
    visibility: VisibilityKind,
    member_name: Option<String>,
    member_short_name: Option<String>,
    /// Optional canonical key — when set, the built element's id is
    /// derived from the key (ADR-009 §Relationships) instead of a
    /// fresh UUID. Used by the parser/elaboration layers to mint
    /// reparse-stable OwningMembership identity.
    canonical_key: Option<CanonicalKey>,
}

impl MembershipBuilder {
    /// Create a new Membership builder.
    pub fn new() -> Self {
        MembershipBuilder {
            kind: ElementKind::Membership,
            member_element: None,
            owning_namespace: None,
            visibility: VisibilityKind::Public,
            member_name: None,
            member_short_name: None,
            canonical_key: None,
        }
    }

    /// Create a new OwningMembership builder.
    pub fn owning() -> Self {
        MembershipBuilder {
            kind: ElementKind::OwningMembership,
            member_element: None,
            owning_namespace: None,
            visibility: VisibilityKind::Public,
            member_name: None,
            member_short_name: None,
            canonical_key: None,
        }
    }

    /// Set the member element.
    pub fn member_element(mut self, id: ElementId) -> Self {
        self.member_element = Some(id);
        self
    }

    /// Set the owning namespace.
    pub fn owning_namespace(mut self, id: ElementId) -> Self {
        self.owning_namespace = Some(id);
        self
    }

    /// Set the visibility.
    pub fn visibility(mut self, vis: VisibilityKind) -> Self {
        self.visibility = vis;
        self
    }

    /// Set the member name.
    pub fn member_name(mut self, name: impl Into<String>) -> Self {
        self.member_name = Some(name.into());
        self
    }

    /// Set the member short name.
    pub fn member_short_name(mut self, name: impl Into<String>) -> Self {
        self.member_short_name = Some(name.into());
        self
    }

    /// Override the membership kind to an `OwningMembership` subtype.
    ///
    /// Used to mint the spec-prescribed membership subtype that wraps a
    /// specific member — e.g. `VariantMembership` for an enumerated value
    /// (SysML `EnumerationUsageMember returns SysML::VariantMembership`,
    /// §8.3.8). The kind must be `OwningMembership` or a subtype so that
    /// [`build`](Self::build) still stamps `ownedMemberElement`.
    pub fn kind(mut self, kind: ElementKind) -> Self {
        debug_assert!(
            kind == ElementKind::OwningMembership
                || kind.is_subtype_of(ElementKind::OwningMembership),
            "MembershipBuilder::kind expects an OwningMembership (sub)kind, got {kind:?}"
        );
        self.kind = kind;
        self
    }

    /// Mint the built Membership/OwningMembership element with a
    /// reparse-stable id derived from a canonical key (ADR-009
    /// §Relationships).
    ///
    /// When this setter is used, [`build`](Self::build) constructs the
    /// element via [`Element::new_with_key`] instead of
    /// [`Element::new_with_kind`]. The key should be derived from the
    /// `(owner, child, kind)` triple via
    /// [`CanonicalKey::for_relationship`].
    pub fn canonical_key(mut self, key: CanonicalKey) -> Self {
        self.canonical_key = Some(key);
        self
    }

    /// Build the Membership element.
    ///
    /// Without [`canonical_key`](Self::canonical_key) this falls back to
    /// a fresh-UUID id — reparse-UNSTABLE. Since the content-true
    /// `ModelGraph::fingerprint` (2026-07-16, plan §W6b), an unstable id
    /// on a parse/elaboration path makes identical re-runs compare
    /// unequal, defeating salsa backdating (a recompute cost, never
    /// stale data). Audited 2026-07-17: every production parse/
    /// elaboration path supplies keys (the tree-sitter builder imports
    /// only `*_with_key` variants; `elaborate/*.rs` non-keyed uses are
    /// test-only); the reachable non-keyed arms are the expression-
    /// element fallbacks when no canonical key context exists. Guarded
    /// by `sysml-ide-db/tests/id_stability.rs` +
    /// `sysml-service/tests/contract_id_stability.rs` — keep new mint
    /// sites keyed.
    pub fn build(self) -> Element {
        let is_owning_membership = self.kind == ElementKind::OwningMembership
            || self.kind.is_subtype_of(ElementKind::OwningMembership);
        let mut element = match self.canonical_key.as_ref() {
            Some(key) => Element::new_with_key(self.kind.clone(), key),
            None => Element::new_with_kind(self.kind),
        };

        if let Some(member) = self.member_element {
            element.props.insert(
                Cow::Borrowed(props::MEMBER_ELEMENT),
                Value::Ref(member.clone()),
            );
            if is_owning_membership {
                element.props.insert(
                    Cow::Borrowed(props::OWNED_MEMBER_ELEMENT),
                    Value::Ref(member),
                );
            }
        }

        if let Some(ns) = self.owning_namespace {
            element.props.insert(
                Cow::Borrowed(props::MEMBERSHIP_OWNING_NAMESPACE),
                Value::Ref(ns),
            );
        }

        element.props.insert(
            Cow::Borrowed(props::VISIBILITY),
            Value::Enum(self.visibility.as_str().to_owned()),
        );

        if let Some(name) = self.member_name {
            element
                .props
                .insert(Cow::Borrowed(props::MEMBER_NAME), Value::String(name));
        }

        if let Some(short_name) = self.member_short_name {
            element.props.insert(
                Cow::Borrowed(props::MEMBER_SHORT_NAME),
                Value::String(short_name),
            );
        }

        element
    }
}

impl Default for MembershipBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Extension trait for Element to provide membership view methods.
impl Element {
    /// Try to view this element as a Membership.
    ///
    /// Returns `Some` if this element is a Membership or subtype.
    pub fn as_membership_view(&self) -> Option<MembershipView<'_>> {
        MembershipView::try_from_element(self)
    }

    /// Try to view this element as an OwningMembership.
    ///
    /// Returns `Some` if this element is an OwningMembership or subtype.
    pub fn as_owning_membership_view(&self) -> Option<OwningMembershipView<'_>> {
        OwningMembershipView::try_from_element(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn membership_builder_basic() {
        let ns_id = ElementId::new_v4();
        let member_id = ElementId::new_v4();

        let membership = MembershipBuilder::new()
            .owning_namespace(ns_id.clone())
            .member_element(member_id.clone())
            .visibility(VisibilityKind::Public)
            .member_name("TestMember")
            .build();

        assert_eq!(membership.kind, ElementKind::Membership);

        let view = membership.as_membership_view().unwrap();
        assert_eq!(view.member_element(), Some(&member_id));
        assert_eq!(view.membership_owning_namespace(), Some(&ns_id));
        assert_eq!(view.visibility(), VisibilityKind::Public);
        assert_eq!(view.member_name(), Some("TestMember"));
        assert!(view.is_public());
    }

    #[test]
    fn owning_membership_builder() {
        let ns_id = ElementId::new_v4();
        let owned_id = ElementId::new_v4();

        let membership = MembershipBuilder::owning()
            .owning_namespace(ns_id.clone())
            .member_element(owned_id.clone())
            .visibility(VisibilityKind::Private)
            .build();

        assert_eq!(membership.kind, ElementKind::OwningMembership);

        let view = membership.as_owning_membership_view().unwrap();
        assert_eq!(view.owned_member_element(), Some(&owned_id));
        assert_eq!(view.membership_owning_namespace(), Some(&ns_id));
        assert_eq!(view.visibility(), VisibilityKind::Private);
    }

    #[test]
    fn membership_view_on_non_membership_returns_none() {
        let element = Element::new_with_kind(ElementKind::Package);
        assert!(element.as_membership_view().is_none());
        assert!(element.as_owning_membership_view().is_none());
    }

    #[test]
    fn visibility_defaults_to_public() {
        let membership = MembershipBuilder::new().build();
        let view = membership.as_membership_view().unwrap();
        assert_eq!(view.visibility(), VisibilityKind::Public);
    }
}
