//! `Element` struct: the core model element type.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use sysml_id::{CanonicalKey, ElementId, QualifiedName};
use sysml_span::Span;

use crate::graph::ModelGraph;
use crate::membership;
use crate::meta::Value;
use crate::ElementKind;

/// A model element.
///
/// ## Ownership Model (SysML v2 Compliant)
///
/// In SysML v2, ownership is established through Membership elements:
/// - `owning_membership` points to the OwningMembership element that owns this element
/// - `owner` is derived from `owning_membership.membershipOwningNamespace`
///
/// For backward compatibility, you can set `owner` directly and an implicit
/// OwningMembership will be created when added to a ModelGraph.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Element {
    /// Unique identifier for this element.
    pub id: ElementId,
    /// The kind of this element.
    pub kind: ElementKind,
    /// The name of this element (optional).
    pub name: Option<String>,
    /// The OwningMembership that owns this element (SysML v2 canonical ownership).
    /// This points to a Membership element, not directly to the owning namespace.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub owning_membership: Option<ElementId>,
    /// The owning element (cached/derived from owning_membership).
    /// This is a convenience field derived from `owning_membership.membershipOwningNamespace`.
    pub owner: Option<ElementId>,
    /// The qualified name of this element (optional, computed).
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub qname: Option<QualifiedName>,
    /// Additional properties.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "BTreeMap::is_empty")
    )]
    pub props: BTreeMap<Cow<'static, str>, Value>,
    /// Source locations for this element.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub spans: Vec<Span>,
    /// Source location of this element's name (narrow span for hover/highlight).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub name_span: Option<Span>,
}

impl Element {
    /// Feed this element's full content into a hasher.
    ///
    /// The one home for "what counts as this element's content":
    /// identity, kind, names, ownership, every property value (via
    /// [`Value::content_hash`] — doc bodies, requirement statements,
    /// attribute defaults, constraint expressions all live in `props`),
    /// and source spans (position consumers like goto-def/hover read
    /// spans off cached graphs, so a span-only shift IS a change).
    /// Backs `ModelGraph::fingerprint`, the salsa change-detection seam
    /// — a field omitted here is a field whose edits get served stale
    /// (the 2026-07-16 doc-text staleness bug).
    pub fn content_hash<H: std::hash::Hasher>(&self, state: &mut H) {
        use std::hash::Hash;
        self.id.hash(state);
        self.kind.hash(state);
        self.name.hash(state);
        self.owning_membership.hash(state);
        self.owner.hash(state);
        self.qname.hash(state);
        self.props.len().hash(state);
        for (key, value) in &self.props {
            key.hash(state);
            value.content_hash(state);
        }
        self.spans.hash(state);
        self.name_span.hash(state);
    }

    /// Create a new element with the given id and kind.
    pub fn new(id: ElementId, kind: ElementKind) -> Self {
        Element {
            id,
            kind,
            name: None,
            owning_membership: None,
            owner: None,
            qname: None,
            props: BTreeMap::new(),
            spans: Vec::new(),
            name_span: None,
        }
    }

    /// Create a new element with a generated id.
    ///
    /// canonical-key: synthetic-fresh-uuid — the reparse-unstable factory
    /// for elaboration / diagram / runtime code that mints synthetic
    /// elements. Parser and elaboration mint sites should prefer
    /// [`Element::new_with_key`] (ADR-009).
    pub fn new_with_kind(kind: ElementKind) -> Self {
        Element::new(ElementId::new_v4(), kind)
    }

    /// Create a new element whose id is derived from a [`CanonicalKey`] (ADR-009).
    ///
    /// Use this in the parse and elaboration layers where reparse-stable
    /// identity matters: the same canonical key always produces the same
    /// `ElementId`, so reparsing the same source file or routing the same
    /// model through a different transport yields byte-identical IDs.
    ///
    /// For synthetic / test elements where reparse stability is not needed,
    /// use [`Element::new_with_kind`], which mints a fresh UUID.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_core::{CanonicalKey, Element, ElementKind};
    ///
    /// let project = CanonicalKey::root("my-project");
    /// let pkg_key = CanonicalKey::for_named(&project, "Package", "Foo");
    /// let elem = Element::new_with_key(ElementKind::Package, &pkg_key);
    /// assert_eq!(elem.id, pkg_key.to_element_id());
    /// ```
    pub fn new_with_key(kind: ElementKind, canonical_key: &CanonicalKey) -> Self {
        Element::new(canonical_key.to_element_id(), kind)
    }

    /// Set the name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the owner.
    pub fn with_owner(mut self, owner: ElementId) -> Self {
        self.owner = Some(owner);
        self
    }

    /// Set the owning membership (SysML v2 canonical ownership).
    ///
    /// This sets the OwningMembership element that owns this element.
    /// Note: You typically don't need to call this directly - use
    /// `ModelGraph::add_owned_element()` which creates the membership for you.
    pub fn with_owning_membership(mut self, membership_id: ElementId) -> Self {
        self.owning_membership = Some(membership_id);
        self
    }

    /// Set the qualified name.
    pub fn with_qname(mut self, qname: QualifiedName) -> Self {
        self.qname = Some(qname);
        self
    }

    /// Add a property.
    pub fn with_prop(mut self, key: impl Into<Cow<'static, str>>, value: impl Into<Value>) -> Self {
        self.props.insert(key.into(), value.into());
        self
    }

    /// Add a span.
    pub fn with_span(mut self, span: Span) -> Self {
        self.spans.push(span);
        self
    }

    /// Get a property value.
    pub fn get_prop(&self, key: &str) -> Option<&Value> {
        self.props.get(key)
    }

    /// Set a property value.
    pub fn set_prop(&mut self, key: impl Into<Cow<'static, str>>, value: impl Into<Value>) {
        self.props.insert(key.into(), value.into());
    }

    /// Compute the effective name of this element.
    ///
    /// The effective name is the name used for name resolution within the owning
    /// namespace. It is derived as follows (per the KerML specification):
    ///
    /// 1. Return the element's own `name` (declared name) if set.
    /// 2. Otherwise, look up the owning Membership and return its `memberName`.
    /// 3. Return `None` if neither is available.
    ///
    /// Spec: Kerml-Vocab.ttl - `name` property:
    /// "The name to be used for this Element during name resolution within its
    /// owningNamespace. This is derived using the effectiveName() operation.
    /// By default, it is the same as the declaredName."
    pub fn effective_name<'a>(&'a self, graph: &'a ModelGraph) -> Option<&'a str> {
        // 1. Own declared name
        if let Some(ref name) = self.name {
            return Some(name.as_str());
        }

        // 2. memberName from owning Membership
        // Access the membership element's props directly to avoid lifetime issues
        // with intermediate MembershipView locals.
        if let Some(ref membership_id) = self.owning_membership {
            if let Some(membership_elem) = graph.get_element(membership_id) {
                if let Some(val) = membership_elem.props.get(membership::props::MEMBER_NAME) {
                    return val.as_str();
                }
            }
        }

        None
    }

    /// Compute the effective short name of this element.
    ///
    /// The effective short name is derived as follows (per the KerML specification):
    ///
    /// 1. Return the element's own `declaredShortName` (stored in props) if set.
    /// 2. Otherwise, look up the owning Membership and return its `memberShortName`.
    /// 3. Return `None` if neither is available.
    ///
    /// Spec: Kerml-Vocab.ttl - `shortName` property:
    /// "The short name to be used for this Element during name resolution within
    /// its owningNamespace. This is derived using the effectiveShortName() operation.
    /// By default, it is the same as the declaredShortName."
    pub fn effective_short_name<'a>(&'a self, graph: &'a ModelGraph) -> Option<&'a str> {
        // 1. Own declared short name (stored as prop "declaredShortName")
        if let Some(val) = self.props.get("declaredShortName") {
            if let Some(s) = val.as_str() {
                return Some(s);
            }
        }

        // 2. memberShortName from owning Membership
        // Access the membership element's props directly to avoid lifetime issues.
        if let Some(ref membership_id) = self.owning_membership {
            if let Some(membership_elem) = graph.get_element(membership_id) {
                if let Some(val) = membership_elem
                    .props
                    .get(membership::props::MEMBER_SHORT_NAME)
                {
                    return val.as_str();
                }
            }
        }

        None
    }
}

impl fmt::Display for Element {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "{} \"{}\"", self.kind.as_str(), name),
            None => write!(f, "{}({})", self.kind.as_str(), self.id),
        }
    }
}

// --- Kind-set predicates (hand-rolled; mirror exact matches! arms at call sites) ---
//
// These are NOT auto-generated. The codegen only produces `is_subtype_of`, which
// is strictly broader and unsafe to use in place of these tight arm sets (e.g.
// `is_subtype_of(ElementKind::ConstraintUsage)` would also match
// RequirementUsage and many sibling kinds, blowing up call-site semantics).
// Each predicate mirrors a verbatim `matches!` arm at the call site it was
// extracted from — keep them byte-identical to those arms.

pub fn is_verification_case_kind(kind: ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::VerificationCaseDefinition | ElementKind::VerificationCaseUsage
    )
}

pub fn is_requirement_kind(kind: ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::RequirementUsage | ElementKind::RequirementDefinition
    )
}

pub fn is_analysis_case_kind(kind: ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::AnalysisCaseDefinition | ElementKind::AnalysisCaseUsage
    )
}

pub fn is_package_kind(kind: ElementKind) -> bool {
    matches!(kind, ElementKind::Package | ElementKind::LibraryPackage)
}

#[cfg(test)]
mod kind_predicate_tests {
    use super::*;

    #[test]
    fn verification_case_kind_predicate() {
        assert!(is_verification_case_kind(
            ElementKind::VerificationCaseDefinition
        ));
        assert!(is_verification_case_kind(
            ElementKind::VerificationCaseUsage
        ));
        assert!(!is_verification_case_kind(ElementKind::PartUsage));
    }

    #[test]
    fn requirement_kind_predicate() {
        assert!(is_requirement_kind(ElementKind::RequirementUsage));
        assert!(is_requirement_kind(ElementKind::RequirementDefinition));
        assert!(!is_requirement_kind(ElementKind::PartUsage));
    }

    #[test]
    fn analysis_case_kind_predicate() {
        assert!(is_analysis_case_kind(ElementKind::AnalysisCaseDefinition));
        assert!(is_analysis_case_kind(ElementKind::AnalysisCaseUsage));
        assert!(!is_analysis_case_kind(
            ElementKind::VerificationCaseDefinition
        ));
    }

    #[test]
    fn package_kind_predicate() {
        assert!(is_package_kind(ElementKind::Package));
        assert!(is_package_kind(ElementKind::LibraryPackage));
        assert!(!is_package_kind(ElementKind::PartUsage));
    }
}
