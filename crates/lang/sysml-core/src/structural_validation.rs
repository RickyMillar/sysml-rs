//! Structural validation for SysML v2 model graphs.
//!
//! This module validates the structural integrity of the model graph:
//! - No orphan elements (except roots)
//! - No ownership cycles
//! - No dangling references in memberships
//! - Relationship type constraints (source/target types)
//!
//! ## Usage
//!
//! ```ignore
//! let errors = graph.validate_structure();
//! if errors.is_empty() {
//!     println!("Model structure is valid!");
//! } else {
//!     for error in errors {
//!         eprintln!("Error: {}", error);
//!     }
//! }
//! ```

use std::collections::HashSet;

#[cfg(feature = "parallel")]
use rayon::prelude::*;

use crate::membership::props as membership_props;
use crate::meta::Value;
use crate::{Element, ElementKind, ModelGraph};
use sysml_id::ElementId;
use sysml_span::{Diagnostic, Span};

/// An error in the structural integrity of the model graph.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum StructuralError {
    /// An element has no owner and is not a valid root type.
    #[error("{}{} must be inside a namespace (package, type, etc.)", element_kind.display_name(), format_optional_name(element_name.as_deref()))]
    OrphanElement {
        element_id: ElementId,
        element_name: Option<String>,
        element_kind: ElementKind,
    },

    /// An ownership cycle was detected.
    #[error("Ownership cycle detected: {element_ids:?}")]
    OwnershipCycle {
        /// The elements forming the cycle (in order).
        element_ids: Vec<ElementId>,
    },

    /// A membership references a non-existent element.
    #[error("Dangling reference in membership {membership_id}: property '{property}' references non-existent element {missing_id}")]
    DanglingMembershipRef {
        membership_id: ElementId,
        /// The property containing the dangling reference.
        property: String,
        /// The missing element ID.
        missing_id: ElementId,
    },

    /// A relationship source element doesn't match expected type.
    #[error("{} expects source to be a {}, but got {}", relationship_kind.display_name(), expected_kind.display_name(), source_kind.display_name())]
    RelationshipSourceTypeMismatch {
        relationship_id: ElementId,
        relationship_kind: ElementKind,
        source_id: ElementId,
        source_kind: ElementKind,
        expected_kind: ElementKind,
    },

    /// A relationship target element doesn't match expected type.
    #[error("{} expects target to be a {}, but got {}", relationship_kind.display_name(), expected_kind.display_name(), target_kind.display_name())]
    RelationshipTargetTypeMismatch {
        relationship_id: ElementId,
        relationship_kind: ElementKind,
        target_id: ElementId,
        target_kind: ElementKind,
        expected_kind: ElementKind,
    },

    /// A relationship references a non-existent element.
    #[error("Dangling {endpoint} reference in relationship {relationship_id}: element {missing_id} does not exist")]
    DanglingRelationshipRef {
        relationship_id: ElementId,
        /// "source" or "target".
        endpoint: String,
        missing_id: ElementId,
    },

    /// An element's owning_membership field points to a non-existent element.
    #[error("Element{} ({}) has owning_membership pointing to non-existent membership {}", format_optional_name(element_name.as_deref()), element_id, missing_membership_id)]
    DanglingOwningMembership {
        element_id: ElementId,
        element_name: Option<String>,
        missing_membership_id: ElementId,
    },

    /// An element's owning_membership points to a non-membership element.
    #[error("element {}'s owning membership {} is a {}, expected membership", element_id, membership_id, membership_kind.display_name())]
    InvalidOwningMembership {
        element_id: ElementId,
        membership_id: ElementId,
        membership_kind: ElementKind,
    },
}

/// Format an optional name as `" 'name'"` or empty string.
fn format_optional_name(name: Option<&str>) -> String {
    match name {
        Some(n) => format!(" '{}'", n),
        None => String::new(),
    }
}

/// Convert StructuralError to Diagnostic for unified error reporting.
///
/// Error codes:
/// - E001: OrphanElement
/// - E002: OwnershipCycle
/// - E003: DanglingMembershipRef
/// - E004: RelationshipSourceTypeMismatch
/// - E005: RelationshipTargetTypeMismatch
/// - E006: DanglingRelationshipRef
/// - E007: DanglingOwningMembership
/// - E008: InvalidOwningMembership
impl From<StructuralError> for sysml_span::Diagnostic {
    fn from(error: StructuralError) -> Self {
        let code = match &error {
            StructuralError::OrphanElement { .. } => "E001",
            StructuralError::OwnershipCycle { .. } => "E002",
            StructuralError::DanglingMembershipRef { .. } => "E003",
            StructuralError::RelationshipSourceTypeMismatch { .. } => "E004",
            StructuralError::RelationshipTargetTypeMismatch { .. } => "E005",
            StructuralError::DanglingRelationshipRef { .. } => "E006",
            StructuralError::DanglingOwningMembership { .. } => "E007",
            StructuralError::InvalidOwningMembership { .. } => "E008",
        };

        sysml_span::Diagnostic::error(error.to_string()).with_code(code.to_owned())
    }
}

impl StructuralError {
    /// Convert this error into a rich Diagnostic with spans and related locations when available.
    pub fn to_diagnostic_with_graph(&self, graph: &ModelGraph) -> Diagnostic {
        let code = match self {
            StructuralError::OrphanElement { .. } => "E001",
            StructuralError::OwnershipCycle { .. } => "E002",
            StructuralError::DanglingMembershipRef { .. } => "E003",
            StructuralError::RelationshipSourceTypeMismatch { .. } => "E004",
            StructuralError::RelationshipTargetTypeMismatch { .. } => "E005",
            StructuralError::DanglingRelationshipRef { .. } => "E006",
            StructuralError::DanglingOwningMembership { .. } => "E007",
            StructuralError::InvalidOwningMembership { .. } => "E008",
        };

        let mut diagnostic = Diagnostic::error(self.to_string()).with_code(code.to_owned());

        match self {
            StructuralError::OrphanElement {
                element_id,
                element_kind,
                ..
            } => {
                if let Some(element) = graph.elements.get(element_id) {
                    diagnostic = attach_primary_span(diagnostic, element.spans.first());
                    diagnostic = diagnostic.with_note(format!(
                        "element: {}",
                        describe_element(element, element_id)
                    ));
                    // Give a specific, actionable hint with example (rust-analyzer style)
                    let (hint, example) = match element_kind {
                        ElementKind::MembershipImport | ElementKind::NamespaceImport => (
                            "move this import inside a package or namespace",
                            "package MyPackage {\n    import SI::*;\n}",
                        ),
                        _ => (
                            "wrap this element in a package or namespace",
                            "package MyPackage {\n    // ... elements go here\n}",
                        ),
                    };
                    diagnostic = diagnostic.with_note(hint);
                    diagnostic = diagnostic.with_note(format!("example:\n{}", example));
                } else {
                    diagnostic = diagnostic.with_note(format!("element id: {}", element_id));
                }
            }
            StructuralError::OwnershipCycle { element_ids } => {
                let mut chain = Vec::new();
                let mut primary_span: Option<Span> = None;
                for id in element_ids {
                    if let Some(element) = graph.elements.get(id) {
                        if primary_span.is_none() {
                            primary_span = element.spans.first().cloned();
                        }
                        chain.push(describe_element(element, id));
                        if let Some(span) = element.spans.first() {
                            diagnostic = diagnostic.with_related(
                                span.clone(),
                                format!("cycle member: {}", describe_element_short(element, id)),
                            );
                        }
                    } else {
                        chain.push(format!("{}", id));
                    }
                }
                if let Some(span) = primary_span {
                    diagnostic = diagnostic.with_span(span);
                }
                if !chain.is_empty() {
                    diagnostic = diagnostic.with_note(format!("cycle: {}", chain.join(" -> ")));
                }
            }
            StructuralError::DanglingMembershipRef {
                membership_id,
                property,
                missing_id,
            } => {
                if let Some(membership) = graph.elements.get(membership_id) {
                    diagnostic = attach_primary_span(diagnostic, membership.spans.first());
                    diagnostic = diagnostic.with_note(format!(
                        "membership: {}",
                        describe_element(membership, membership_id)
                    ));
                    if let Some(owner_id) = &membership.owner {
                        if let Some(owner) = graph.elements.get(owner_id) {
                            if let Some(span) = owner.spans.first() {
                                diagnostic = diagnostic.with_related(
                                    span.clone(),
                                    format!(
                                        "owning namespace: {}",
                                        describe_element_short(owner, owner_id)
                                    ),
                                );
                            }
                        }
                    }
                }
                diagnostic = diagnostic.with_notes([
                    format!("property: {}", property),
                    format!("missing element id: {}", missing_id),
                ]);
            }
            StructuralError::RelationshipSourceTypeMismatch {
                relationship_id,
                relationship_kind,
                source_id,
                source_kind,
                expected_kind,
            } => {
                if let Some(rel_elem) = graph.elements.get(relationship_id) {
                    diagnostic = attach_primary_span(diagnostic, rel_elem.spans.first());
                    diagnostic = diagnostic.with_note(format!(
                        "relationship element: {}",
                        describe_element(rel_elem, relationship_id)
                    ));
                } else {
                    diagnostic = diagnostic.with_note(format!(
                        "relationship id: {} ({})",
                        relationship_id,
                        relationship_kind.display_name()
                    ));
                }
                if let Some(source_elem) = graph.elements.get(source_id) {
                    if let Some(span) = source_elem.spans.first() {
                        diagnostic = diagnostic.with_related(
                            span.clone(),
                            format!(
                                "source element: {}",
                                describe_element_short(source_elem, source_id)
                            ),
                        );
                    }
                }
                diagnostic = diagnostic.with_notes([
                    format!("expected source kind: {}", expected_kind.display_name()),
                    format!("actual source kind: {}", source_kind.display_name()),
                ]);
            }
            StructuralError::RelationshipTargetTypeMismatch {
                relationship_id,
                relationship_kind,
                target_id,
                target_kind,
                expected_kind,
            } => {
                if let Some(rel_elem) = graph.elements.get(relationship_id) {
                    diagnostic = attach_primary_span(diagnostic, rel_elem.spans.first());
                    diagnostic = diagnostic.with_note(format!(
                        "relationship element: {}",
                        describe_element(rel_elem, relationship_id)
                    ));
                } else {
                    diagnostic = diagnostic.with_note(format!(
                        "relationship id: {} ({})",
                        relationship_id,
                        relationship_kind.display_name()
                    ));
                }
                if let Some(target_elem) = graph.elements.get(target_id) {
                    if let Some(span) = target_elem.spans.first() {
                        diagnostic = diagnostic.with_related(
                            span.clone(),
                            format!(
                                "target element: {}",
                                describe_element_short(target_elem, target_id)
                            ),
                        );
                    }
                }
                diagnostic = diagnostic.with_notes([
                    format!("expected target kind: {}", expected_kind.display_name()),
                    format!("actual target kind: {}", target_kind.display_name()),
                ]);
            }
            StructuralError::DanglingRelationshipRef {
                relationship_id,
                endpoint,
                missing_id,
            } => {
                if let Some(rel_elem) = graph.elements.get(relationship_id) {
                    diagnostic = attach_primary_span(diagnostic, rel_elem.spans.first());
                    diagnostic = diagnostic.with_note(format!(
                        "relationship element: {}",
                        describe_element(rel_elem, relationship_id)
                    ));
                } else {
                    diagnostic =
                        diagnostic.with_note(format!("relationship id: {}", relationship_id));
                }

                if let Some(rel) = graph.relationships.get(relationship_id) {
                    if let Some(source) = graph.elements.get(&rel.source) {
                        if let Some(span) = source.spans.first() {
                            diagnostic = diagnostic.with_related(
                                span.clone(),
                                format!(
                                    "source element: {}",
                                    describe_element_short(source, &rel.source)
                                ),
                            );
                        }
                    }
                    if let Some(target) = graph.elements.get(&rel.target) {
                        if let Some(span) = target.spans.first() {
                            diagnostic = diagnostic.with_related(
                                span.clone(),
                                format!(
                                    "target element: {}",
                                    describe_element_short(target, &rel.target)
                                ),
                            );
                        }
                    }
                }

                diagnostic = diagnostic.with_notes([
                    format!("missing {} id: {}", endpoint, missing_id),
                    "relationship references must point to existing elements".to_owned(),
                ]);
            }
            StructuralError::DanglingOwningMembership {
                element_id,
                element_name: _,
                missing_membership_id,
            } => {
                if let Some(element) = graph.elements.get(element_id) {
                    diagnostic = attach_primary_span(diagnostic, element.spans.first());
                    diagnostic = diagnostic.with_note(format!(
                        "element: {}",
                        describe_element(element, element_id)
                    ));
                }
                diagnostic = diagnostic.with_note(format!(
                    "missing owning_membership id: {}",
                    missing_membership_id
                ));
            }
            StructuralError::InvalidOwningMembership {
                element_id,
                membership_id,
                membership_kind,
            } => {
                if let Some(element) = graph.elements.get(element_id) {
                    diagnostic = attach_primary_span(diagnostic, element.spans.first());
                    diagnostic = diagnostic.with_note(format!(
                        "element: {}",
                        describe_element(element, element_id)
                    ));
                }
                if let Some(membership) = graph.elements.get(membership_id) {
                    if let Some(span) = membership.spans.first() {
                        diagnostic = diagnostic.with_related(
                            span.clone(),
                            format!(
                                "owning_membership element: {}",
                                describe_element_short(membership, membership_id)
                            ),
                        );
                    }
                }
                diagnostic = diagnostic.with_note(format!(
                    "owning_membership must be a membership, found {}",
                    membership_kind.display_name()
                ));
            }
        }

        diagnostic
    }
}

fn attach_primary_span(mut diagnostic: Diagnostic, span: Option<&Span>) -> Diagnostic {
    if let Some(span) = span {
        diagnostic = diagnostic.with_span(span.clone());
    }
    diagnostic
}

fn describe_element(element: &Element, id: &ElementId) -> String {
    match &element.name {
        Some(name) => format!("{} '{}' ({})", element.kind.display_name(), name, id),
        None => format!("{} ({})", element.kind.display_name(), id),
    }
}

fn describe_element_short(element: &Element, id: &ElementId) -> String {
    match &element.name {
        Some(name) => format!("{} '{}'", element.kind.display_name(), name),
        None => format!("{} ({})", element.kind.display_name(), id),
    }
}

/// Types that are valid as root elements (can exist without an owner).
const VALID_ROOT_KINDS: &[ElementKind] = &[
    ElementKind::Package,
    ElementKind::LibraryPackage,
    // Membership elements can be roots in certain contexts
    ElementKind::Membership,
    ElementKind::OwningMembership,
    ElementKind::FeatureMembership,
];

/// Check if an element kind is valid as a root (no owner).
fn is_valid_root_kind(kind: &ElementKind) -> bool {
    VALID_ROOT_KINDS.contains(kind)
        || kind.is_subtype_of(ElementKind::Membership)
        || kind.is_subtype_of(ElementKind::Package)
}

/// Check if an element kind is compatible with an expected kind.
///
/// An element kind is compatible if it either:
/// 1. Is exactly the expected kind, or
/// 2. Is a subtype of the expected kind
///
/// # Examples
///
/// - PartUsage is compatible with Feature (subtype)
/// - Feature is compatible with Feature (exact match)
/// - Package is NOT compatible with Feature (unrelated)
fn is_compatible_kind(actual: &ElementKind, expected: &ElementKind) -> bool {
    actual == expected || actual.is_subtype_of(expected.clone())
}

impl ModelGraph {
    /// Validate the structural integrity of the model graph.
    ///
    /// Checks for:
    /// - Orphan elements (non-root elements without owners)
    /// - Ownership cycles
    /// - Dangling references in memberships
    /// - Invalid owning_membership references
    ///
    /// Performance: For graphs with >5000 elements, runs all 5 validation passes
    /// in parallel using rayon, providing 3-5x speedup on multi-core systems.
    /// For smaller graphs, runs sequentially to avoid thread pool overhead.
    ///
    /// # Returns
    ///
    /// A vector of structural errors. Empty if the model is valid.
    pub fn validate_structure(&self) -> Vec<StructuralError> {
        // Parallel overhead isn't worth it for small graphs
        // Threshold determined empirically from benchmarks
        const PARALLEL_THRESHOLD: usize = 5000;

        if self.elements.len() >= PARALLEL_THRESHOLD {
            self.validate_structure_parallel()
        } else {
            self.validate_structure_sequential()
        }
    }

    /// Sequential validation for small graphs (avoids rayon overhead).
    fn validate_structure_sequential(&self) -> Vec<StructuralError> {
        let mut errors = self.collect_orphan_errors();
        errors.extend(self.collect_ownership_cycle_errors());
        errors.extend(self.collect_membership_reference_errors());
        errors.extend(self.collect_owning_membership_reference_errors());
        errors.extend(self.collect_relationship_reference_errors());
        errors
    }

    /// Parallel validation for large graphs using rayon.
    #[cfg(feature = "parallel")]
    fn validate_structure_parallel(&self) -> Vec<StructuralError> {
        // Run all validation passes in parallel using rayon::join
        // Each pass is read-only (&self), so they're thread-safe
        let (left_results, right_results) = rayon::join(
            || {
                rayon::join(
                    || self.collect_orphan_errors(),
                    || self.collect_ownership_cycle_errors(),
                )
            },
            || {
                rayon::join(
                    || {
                        rayon::join(
                            || self.collect_membership_reference_errors(),
                            || self.collect_owning_membership_reference_errors(),
                        )
                    },
                    || self.collect_relationship_reference_errors(),
                )
            },
        );

        // Combine all error vectors
        let mut errors = left_results.0;
        errors.extend(left_results.1);
        errors.extend(right_results.0 .0);
        errors.extend(right_results.0 .1);
        errors.extend(right_results.1);
        errors
    }

    /// Fallback when rayon is not available — delegates to sequential.
    #[cfg(not(feature = "parallel"))]
    fn validate_structure_parallel(&self) -> Vec<StructuralError> {
        self.validate_structure_sequential()
    }

    /// Check for orphan elements and return errors.
    #[cfg(feature = "parallel")]
    fn collect_orphan_errors(&self) -> Vec<StructuralError> {
        self.elements
            .par_iter()
            .filter(|(_, element)| {
                element.owner.is_none()
                    && element.owning_membership.is_none()
                    && !is_valid_root_kind(&element.kind)
            })
            .map(|(id, element)| StructuralError::OrphanElement {
                element_id: id.clone(),
                element_name: element.name.clone(),
                element_kind: element.kind.clone(),
            })
            .collect()
    }

    /// Check for orphan elements and return errors (sequential).
    #[cfg(not(feature = "parallel"))]
    fn collect_orphan_errors(&self) -> Vec<StructuralError> {
        self.elements
            .iter()
            .filter(|(_, element)| {
                element.owner.is_none()
                    && element.owning_membership.is_none()
                    && !is_valid_root_kind(&element.kind)
            })
            .map(|(id, element)| StructuralError::OrphanElement {
                element_id: id.clone(),
                element_name: element.name.clone(),
                element_kind: element.kind.clone(),
            })
            .collect()
    }

    /// Check for ownership cycles and return errors.
    #[allow(clippy::unwrap_used, clippy::indexing_slicing)] // invariant: path_set membership guarantees position exists
    fn collect_ownership_cycle_errors(&self) -> Vec<StructuralError> {
        let mut errors = Vec::new();
        let mut visited_global: HashSet<ElementId> = HashSet::new();

        for id in self.elements.keys() {
            if visited_global.contains(id) {
                continue;
            }

            let mut path: Vec<ElementId> = Vec::new();
            let mut path_set: HashSet<ElementId> = HashSet::new();
            let mut current = Some(id.clone());

            while let Some(current_id) = current {
                if path_set.contains(&current_id) {
                    // Found a cycle - extract the cycle portion
                    let cycle_start = path.iter().position(|i| i == &current_id).unwrap();
                    let cycle: Vec<ElementId> = path[cycle_start..].to_vec();
                    errors.push(StructuralError::OwnershipCycle { element_ids: cycle });
                    break;
                }

                visited_global.insert(current_id.clone());
                path.push(current_id.clone());
                path_set.insert(current_id.clone());

                // Move to owner
                current = self.elements.get(&current_id).and_then(|e| e.owner.clone());
            }
        }
        errors
    }

    /// Validate membership element references and return errors.
    #[cfg(feature = "parallel")]
    fn collect_membership_reference_errors(&self) -> Vec<StructuralError> {
        self.elements
            .par_iter()
            .filter(|(_, element)| {
                element.kind == ElementKind::Membership
                    || element.kind.is_subtype_of(ElementKind::Membership)
            })
            .flat_map(|(id, element)| self.check_membership_references(id, element))
            .collect()
    }

    /// Validate membership element references and return errors (sequential).
    #[cfg(not(feature = "parallel"))]
    fn collect_membership_reference_errors(&self) -> Vec<StructuralError> {
        self.elements
            .iter()
            .filter(|(_, element)| {
                element.kind == ElementKind::Membership
                    || element.kind.is_subtype_of(ElementKind::Membership)
            })
            .flat_map(|(id, element)| self.check_membership_references(id, element))
            .collect()
    }

    /// Check a single membership element for dangling references.
    fn check_membership_references(
        &self,
        id: &ElementId,
        element: &Element,
    ) -> Vec<StructuralError> {
        let mut errors = Vec::new();

        // Check memberElement reference
        if let Some(member_ref) = element.props.get(membership_props::MEMBER_ELEMENT) {
            if let Some(member_id) = member_ref.as_ref() {
                if !self.elements.contains_key(member_id) {
                    errors.push(StructuralError::DanglingMembershipRef {
                        membership_id: id.clone(),
                        property: membership_props::MEMBER_ELEMENT.to_owned(),
                        missing_id: member_id.clone(),
                    });
                }
            }
        }

        // Check membershipOwningNamespace reference
        if let Some(ns_ref) = element
            .props
            .get(membership_props::MEMBERSHIP_OWNING_NAMESPACE)
        {
            if let Some(ns_id) = ns_ref.as_ref() {
                if !self.elements.contains_key(ns_id) {
                    errors.push(StructuralError::DanglingMembershipRef {
                        membership_id: id.clone(),
                        property: membership_props::MEMBERSHIP_OWNING_NAMESPACE.to_owned(),
                        missing_id: ns_id.clone(),
                    });
                }
            }
        }

        errors
    }

    /// Validate owning_membership references in elements and return errors.
    #[cfg(feature = "parallel")]
    fn collect_owning_membership_reference_errors(&self) -> Vec<StructuralError> {
        self.elements
            .par_iter()
            .filter_map(|(id, element)| self.check_owning_membership_reference(id, element))
            .collect()
    }

    /// Validate owning_membership references in elements and return errors (sequential).
    #[cfg(not(feature = "parallel"))]
    fn collect_owning_membership_reference_errors(&self) -> Vec<StructuralError> {
        self.elements
            .iter()
            .filter_map(|(id, element)| self.check_owning_membership_reference(id, element))
            .collect()
    }

    /// Check a single element's owning_membership reference.
    fn check_owning_membership_reference(
        &self,
        id: &ElementId,
        element: &Element,
    ) -> Option<StructuralError> {
        let membership_id = element.owning_membership.as_ref()?;
        match self.elements.get(membership_id) {
            None => Some(StructuralError::DanglingOwningMembership {
                element_id: id.clone(),
                element_name: element.name.clone(),
                missing_membership_id: membership_id.clone(),
            }),
            Some(membership) => {
                if membership.kind != ElementKind::Membership
                    && !membership.kind.is_subtype_of(ElementKind::Membership)
                {
                    Some(StructuralError::InvalidOwningMembership {
                        element_id: id.clone(),
                        membership_id: membership_id.clone(),
                        membership_kind: membership.kind.clone(),
                    })
                } else {
                    None
                }
            }
        }
    }

    /// Validate relationship source/target references and return errors.
    #[cfg(feature = "parallel")]
    fn collect_relationship_reference_errors(&self) -> Vec<StructuralError> {
        self.relationships
            .par_iter()
            .flat_map(|(id, rel)| self.check_relationship_references(id, rel))
            .collect()
    }

    /// Validate relationship source/target references and return errors (sequential).
    #[cfg(not(feature = "parallel"))]
    fn collect_relationship_reference_errors(&self) -> Vec<StructuralError> {
        self.relationships
            .iter()
            .flat_map(|(id, rel)| self.check_relationship_references(id, rel))
            .collect()
    }

    /// Check a single relationship for dangling source/target references.
    fn check_relationship_references(
        &self,
        id: &ElementId,
        rel: &crate::Relationship,
    ) -> Vec<StructuralError> {
        let mut errors = Vec::new();

        // Check source exists
        if !self.elements.contains_key(&rel.source) {
            errors.push(StructuralError::DanglingRelationshipRef {
                relationship_id: id.clone(),
                endpoint: "source".to_owned(),
                missing_id: rel.source.clone(),
            });
        }

        // Check target exists
        if !self.elements.contains_key(&rel.target) {
            errors.push(StructuralError::DanglingRelationshipRef {
                relationship_id: id.clone(),
                endpoint: "target".to_owned(),
                missing_id: rel.target.clone(),
            });
        }

        errors
    }

    /// Validate relationship type constraints for Element-based relationships.
    ///
    /// This checks that Relationship elements (like FeatureTyping, Specialization)
    /// have source/target types matching the spec constraints derived from the XMI metamodel.
    ///
    /// ## Validation Rules
    ///
    /// For each relationship element:
    /// 1. **Source validation**: The owner element's kind must be compatible with the
    ///    expected source type (e.g., FeatureTyping requires a Feature owner)
    /// 2. **Target validation**: The element referenced by the target property must have
    ///    a kind compatible with the expected target type (e.g., FeatureTyping.type must
    ///    reference a Type)
    ///
    /// ## Property Resolution
    ///
    /// Target elements are found using `ElementKind::relationship_target_property()` which
    /// returns the property name containing the target reference (e.g., "general" for
    /// Specialization, "type" for FeatureTyping).
    ///
    /// ## Note
    ///
    /// This validation should be called after name resolution, when target properties
    /// contain resolved ElementIds rather than string references.
    pub fn validate_relationship_types(&self) -> Vec<StructuralError> {
        let mut errors = Vec::new();

        for (id, element) in &self.elements {
            // Only check Relationship elements
            if !element.kind.is_relationship() {
                continue;
            }

            // === Validate Source ===
            if let Some(expected_source_kind) = element.kind.relationship_source_type() {
                // Most relationship sources are represented by the owner element.
                // SuccessionAsUsage is special: in forms like `entry; then Off;`, the source
                // can be implicit/empty and owner is a container StateDefinition, not the
                // semantic source endpoint. In that case, skip source-kind validation.
                let source_id = if element.kind == ElementKind::SuccessionAsUsage {
                    element
                        .props
                        .get("source")
                        .and_then(|v| v.as_ref())
                        .cloned()
                } else {
                    element.owner.clone()
                };

                if let Some(source_id) = source_id {
                    if let Some(source) = self.elements.get(&source_id) {
                        // Skip source validation for relationships inside connection
                        // definitions, interface definitions, and allocation definitions.
                        // These types have structural connection end features (e.g.,
                        // `end source : Port`) whose ownership relationships don't
                        // follow the normal source type constraints.
                        //
                        // Also skip when a ConnectionUsage/FlowUsage is owned by any
                        // definition/usage — the semantic source is the feature chain
                        // endpoint (e.g., `waterTank.waterOut`), not the containing
                        // part definition.
                        // Skip when owner is a membership type — the typing
                        // relationship is on the membership wrapper, not the
                        // semantic feature.  E.g. `return time : Real;` creates
                        // ReturnParameterMembership owning a FeatureTyping.
                        let owner_is_membership =
                            source.kind.is_subtype_of(ElementKind::Membership)
                                || source.kind == ElementKind::Membership;
                        let owner_is_connection_like = matches!(
                            source.kind,
                            ElementKind::ConnectionDefinition
                                | ElementKind::InterfaceDefinition
                                | ElementKind::AllocationDefinition
                                | ElementKind::ConnectionUsage
                                | ElementKind::InterfaceUsage
                                | ElementKind::AllocationUsage
                                | ElementKind::ConnectorAsUsage
                                | ElementKind::BindingConnectorAsUsage
                                | ElementKind::SuccessionAsUsage
                        );
                        let is_connection_in_container = matches!(
                            element.kind,
                            ElementKind::ConnectionUsage
                                | ElementKind::FlowUsage
                                | ElementKind::SuccessionFlowUsage
                                | ElementKind::InterfaceUsage
                                | ElementKind::AllocationUsage
                                | ElementKind::ConnectorAsUsage
                                | ElementKind::BindingConnectorAsUsage
                                | ElementKind::SuccessionAsUsage
                        ) && (source.kind.is_definition()
                            || source.kind.is_usage());
                        // ResultExpressionMembership: XMI says source=Function, but KerML
                        // spec also allows Expression (which ConstraintUsage inherits via
                        // BooleanExpression). The semantic check S051 handles the full rule.
                        let is_result_expr_in_expression = element.kind
                            == ElementKind::ResultExpressionMembership
                            && source.kind.is_subtype_of(ElementKind::Expression);
                        if !owner_is_membership
                            && !owner_is_connection_like
                            && !is_connection_in_container
                            && !is_result_expr_in_expression
                            && !is_compatible_kind(&source.kind, &expected_source_kind)
                        {
                            errors.push(StructuralError::RelationshipSourceTypeMismatch {
                                relationship_id: id.clone(),
                                relationship_kind: element.kind.clone(),
                                source_id,
                                source_kind: source.kind.clone(),
                                expected_kind: expected_source_kind.clone(),
                            });
                        }
                    }
                    // Note: Missing source elements are caught by dangling relationship validation
                }
            }

            // === Validate Target (from property) ===
            if let Some(expected_target_kind) = element.kind.relationship_target_type() {
                // Get the property name containing the target reference
                if let Some(prop_name) = element.kind.relationship_target_property() {
                    if element.kind.relationship_target_is_list() {
                        // List property (e.g., Dependency.supplier)
                        self.validate_list_target(
                            &mut errors,
                            id.clone(),
                            element,
                            prop_name,
                            expected_target_kind,
                        );
                    } else {
                        // Single target property
                        self.validate_single_target(
                            &mut errors,
                            id.clone(),
                            element,
                            prop_name,
                            expected_target_kind,
                        );
                    }
                }
                // Note: Relationships without target property mapping skip target validation
            }
        }

        errors
    }

    /// Validate a single-valued target property.
    fn validate_single_target(
        &self,
        errors: &mut Vec<StructuralError>,
        rel_id: ElementId,
        element: &Element,
        prop_name: &str,
        expected_kind: ElementKind,
    ) {
        // Get target ElementId from the property value (if resolved)
        let target_id: Option<&ElementId> = element.props.get(prop_name).and_then(|v| v.as_ref());

        if let Some(tid) = target_id {
            if let Some(target) = self.elements.get(tid) {
                if !is_compatible_kind(&target.kind, &expected_kind) {
                    errors.push(StructuralError::RelationshipTargetTypeMismatch {
                        relationship_id: rel_id,
                        relationship_kind: element.kind.clone(),
                        target_id: tid.clone(),
                        target_kind: target.kind.clone(),
                        expected_kind,
                    });
                }
            }
            // Note: Missing target element is caught by dangling reference validation
        }
        // Note: If the value isn't a resolved reference (still a string), skip validation
        // This handles the case where resolution hasn't completed
    }

    /// Validate a list-valued target property.
    #[allow(clippy::needless_pass_by_value)] // ElementKind is not Copy due to #[non_exhaustive]
    fn validate_list_target(
        &self,
        errors: &mut Vec<StructuralError>,
        rel_id: ElementId,
        element: &Element,
        prop_name: &str,
        expected_kind: ElementKind,
    ) {
        // Get the list property value
        let list_value: Option<&Vec<Value>> =
            element.props.get(prop_name).and_then(|v| v.as_list());

        if let Some(list) = list_value {
            for item in list {
                if let Some(target_id) = item.as_ref() {
                    if let Some(target) = self.elements.get(target_id) {
                        if !is_compatible_kind(&target.kind, &expected_kind) {
                            errors.push(StructuralError::RelationshipTargetTypeMismatch {
                                relationship_id: rel_id.clone(),
                                relationship_kind: element.kind.clone(),
                                target_id: target_id.clone(),
                                target_kind: target.kind.clone(),
                                expected_kind: expected_kind.clone(),
                            });
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Element, VisibilityKind};

    #[test]
    fn valid_structure_no_errors() {
        let mut graph = ModelGraph::new();

        // Create a valid hierarchy
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let part = Element::new_with_kind(ElementKind::PartDefinition).with_name("Part");
        let _part_id = graph.add_owned_element(part, pkg_id.clone(), VisibilityKind::Public);

        let errors = graph.validate_structure();
        assert!(errors.is_empty(), "Expected no errors, got: {:?}", errors);
    }

    #[test]
    fn orphan_element_detected() {
        let mut graph = ModelGraph::new();

        // Add a PartDefinition without an owner (not a valid root)
        let part = Element::new_with_kind(ElementKind::PartDefinition).with_name("OrphanPart");
        graph.add_element(part);

        let errors = graph.validate_structure();
        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0], StructuralError::OrphanElement { .. }));
    }

    #[test]
    fn package_can_be_root() {
        let mut graph = ModelGraph::new();

        // Package is a valid root
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("RootPackage");
        graph.add_element(pkg);

        let errors = graph.validate_structure();
        assert!(errors.is_empty());
    }

    #[test]
    fn ownership_cycle_detected() {
        let mut graph = ModelGraph::new();

        // Create elements manually to create a cycle
        let a = Element::new_with_kind(ElementKind::Package).with_name("A");
        let a_id = a.id.clone();
        graph.add_element(a);

        let b = Element::new_with_kind(ElementKind::Package)
            .with_name("B")
            .with_owner(a_id.clone());
        let b_id = b.id.clone();
        graph.add_element(b);

        // Create cycle: set A's owner to B
        if let Some(a_elem) = graph.elements.get_mut(&a_id) {
            a_elem.owner = Some(b_id);
        }

        let errors = graph.validate_structure();
        let cycle_errors: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, StructuralError::OwnershipCycle { .. }))
            .collect();
        assert!(!cycle_errors.is_empty(), "Expected cycle error");
    }

    #[test]
    fn dangling_owning_membership_detected() {
        let mut graph = ModelGraph::new();

        // Create an element with a dangling owning_membership
        let fake_membership_id = ElementId::new_v4();
        let part = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("Part")
            .with_owning_membership(fake_membership_id.clone());
        graph.add_element(part);

        let errors = graph.validate_structure();
        let dangling_errors: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, StructuralError::DanglingOwningMembership { .. }))
            .collect();
        assert!(
            !dangling_errors.is_empty(),
            "Expected dangling membership error"
        );
    }

    #[test]
    fn invalid_owning_membership_type_detected() {
        let mut graph = ModelGraph::new();

        // Create a package (not a Membership)
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        // Create an element pointing to Package as owning_membership (wrong!)
        let part = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("Part")
            .with_owning_membership(pkg_id);
        graph.add_element(part);

        let errors = graph.validate_structure();
        let invalid_errors: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, StructuralError::InvalidOwningMembership { .. }))
            .collect();
        assert!(
            !invalid_errors.is_empty(),
            "Expected invalid membership type error"
        );
    }

    #[test]
    fn structural_error_display() {
        let error = StructuralError::OrphanElement {
            element_id: ElementId::new_v4(),
            element_name: Some("TestPart".to_string()),
            element_kind: ElementKind::PartDefinition,
        };

        let msg = format!("{}", error);
        assert!(msg.contains("must be inside a namespace"));
        assert!(msg.contains("part definition"));
        assert!(msg.contains("TestPart"));
    }

    // === Relationship Type Validation Tests (Phase 4) ===

    #[test]
    fn valid_feature_typing_passes() {
        let mut graph = ModelGraph::new();

        // Create a Feature (owner/source)
        let feature = Element::new_with_kind(ElementKind::Feature).with_name("myFeature");
        let feature_id = graph.add_element(feature);

        // Create a Type (target)
        let type_elem = Element::new_with_kind(ElementKind::Type).with_name("MyType");
        let type_id = graph.add_element(type_elem);

        // Create a FeatureTyping relationship owned by the Feature
        let typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_owner(feature_id)
            .with_prop("type", Value::Ref(type_id));
        graph.add_element(typing);

        let errors = graph.validate_relationship_types();
        assert!(
            errors.is_empty(),
            "Valid FeatureTyping should pass: {:?}",
            errors
        );
    }

    #[test]
    fn feature_typing_source_mismatch_detected() {
        let mut graph = ModelGraph::new();

        // Create a Package (NOT a Feature - wrong source type)
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("NotAFeature");
        let pkg_id = graph.add_element(pkg);

        // Create a Type (target)
        let type_elem = Element::new_with_kind(ElementKind::Type).with_name("MyType");
        let type_id = graph.add_element(type_elem);

        // Create a FeatureTyping relationship owned by Package (wrong!)
        let typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_owner(pkg_id.clone())
            .with_prop("type", Value::Ref(type_id));
        graph.add_element(typing);

        let errors = graph.validate_relationship_types();
        let source_errors: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, StructuralError::RelationshipSourceTypeMismatch { .. }))
            .collect();
        assert!(
            !source_errors.is_empty(),
            "Expected source type mismatch error for FeatureTyping owned by Package"
        );
    }

    #[test]
    fn feature_typing_target_mismatch_detected() {
        let mut graph = ModelGraph::new();

        // Create a Feature (valid source)
        let feature = Element::new_with_kind(ElementKind::Feature).with_name("myFeature");
        let feature_id = graph.add_element(feature);

        // Create a Relationship (NOT a Type - wrong target type)
        let wrong_target = Element::new_with_kind(ElementKind::Relationship).with_name("NotAType");
        let wrong_target_id = graph.add_element(wrong_target);

        // Create a FeatureTyping with wrong target type
        let typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_owner(feature_id)
            .with_prop("type", Value::Ref(wrong_target_id));
        graph.add_element(typing);

        let errors = graph.validate_relationship_types();
        let target_errors: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, StructuralError::RelationshipTargetTypeMismatch { .. }))
            .collect();
        assert!(
            !target_errors.is_empty(),
            "Expected target type mismatch error for FeatureTyping targeting Relationship"
        );
    }

    #[test]
    fn valid_specialization_passes() {
        let mut graph = ModelGraph::new();

        // Create a Type (source/specific)
        let specific = Element::new_with_kind(ElementKind::Type).with_name("SpecificType");
        let specific_id = graph.add_element(specific);

        // Create another Type (target/general)
        let general = Element::new_with_kind(ElementKind::Type).with_name("GeneralType");
        let general_id = graph.add_element(general);

        // Create a Specialization relationship
        let spec = Element::new_with_kind(ElementKind::Specialization)
            .with_owner(specific_id)
            .with_prop("general", Value::Ref(general_id));
        graph.add_element(spec);

        let errors = graph.validate_relationship_types();
        assert!(
            errors.is_empty(),
            "Valid Specialization should pass: {:?}",
            errors
        );
    }

    #[test]
    fn specialization_target_mismatch_detected() {
        let mut graph = ModelGraph::new();

        // Create a Type (valid source)
        let specific = Element::new_with_kind(ElementKind::Type).with_name("SpecificType");
        let specific_id = graph.add_element(specific);

        // Create an Element (NOT a Type - wrong target)
        let wrong_target = Element::new_with_kind(ElementKind::Element).with_name("NotAType");
        let wrong_target_id = graph.add_element(wrong_target);

        // Create a Specialization with wrong target type
        let spec = Element::new_with_kind(ElementKind::Specialization)
            .with_owner(specific_id)
            .with_prop("general", Value::Ref(wrong_target_id));
        graph.add_element(spec);

        let errors = graph.validate_relationship_types();
        let target_errors: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, StructuralError::RelationshipTargetTypeMismatch { .. }))
            .collect();
        assert!(
            !target_errors.is_empty(),
            "Expected target type mismatch for Specialization targeting Element"
        );
    }

    #[test]
    fn subtype_is_compatible_with_expected_type() {
        let mut graph = ModelGraph::new();

        // Create a PartUsage (which is a subtype of Feature)
        let part = Element::new_with_kind(ElementKind::PartUsage).with_name("myPart");
        let part_id = graph.add_element(part);

        // Create a Classifier (which is a subtype of Type)
        let classifier = Element::new_with_kind(ElementKind::Classifier).with_name("MyClassifier");
        let classifier_id = graph.add_element(classifier);

        // Create a FeatureTyping: source=PartUsage (subtype of Feature), target=Classifier (subtype of Type)
        let typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_owner(part_id)
            .with_prop("type", Value::Ref(classifier_id));
        graph.add_element(typing);

        let errors = graph.validate_relationship_types();
        assert!(
            errors.is_empty(),
            "Subtypes should be compatible: {:?}",
            errors
        );
    }

    #[test]
    fn unresolved_target_skipped() {
        let mut graph = ModelGraph::new();

        // Create a Feature
        let feature = Element::new_with_kind(ElementKind::Feature).with_name("myFeature");
        let feature_id = graph.add_element(feature);

        // Create a FeatureTyping with unresolved target (string, not Ref)
        let typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_owner(feature_id)
            .with_prop("type", Value::String("UnresolvedTypeName".to_string()));
        graph.add_element(typing);

        let errors = graph.validate_relationship_types();
        // Should NOT produce an error - unresolved targets are skipped
        let target_errors: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, StructuralError::RelationshipTargetTypeMismatch { .. }))
            .collect();
        assert!(
            target_errors.is_empty(),
            "Unresolved targets should be skipped"
        );
    }

    #[test]
    fn relationship_without_target_property_skipped() {
        let mut graph = ModelGraph::new();

        // Create a Package
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        // Create a base Relationship (no target property mapping)
        let rel = Element::new_with_kind(ElementKind::Relationship).with_owner(pkg_id);
        graph.add_element(rel);

        let errors = graph.validate_relationship_types();
        // Base Relationship types without property mappings should not cause target errors
        let target_errors: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, StructuralError::RelationshipTargetTypeMismatch { .. }))
            .collect();
        assert!(
            target_errors.is_empty(),
            "Relationships without target property should be skipped"
        );
    }

    #[test]
    fn succession_as_usage_with_implicit_source_skips_owner_source_validation() {
        let mut graph = ModelGraph::new();

        // Valid container for a state machine body item.
        let state_def = Element::new_with_kind(ElementKind::StateDefinition).with_name("Toggle");
        let state_def_id = graph.add_element(state_def);

        // SuccessionAsUsage without an explicit "source" corresponds to forms like:
        //   entry; then Off;
        let succession = Element::new_with_kind(ElementKind::SuccessionAsUsage)
            .with_owner(state_def_id)
            .with_prop("target", Value::String("Off".to_string()));
        graph.add_element(succession);

        let errors = graph.validate_relationship_types();
        let source_errors: Vec<_> = errors
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    StructuralError::RelationshipSourceTypeMismatch {
                        relationship_kind: ElementKind::SuccessionAsUsage,
                        ..
                    }
                )
            })
            .collect();
        assert!(
            source_errors.is_empty(),
            "Implicit-source SuccessionAsUsage should not use owner for source validation"
        );
    }

    #[test]
    fn succession_as_usage_with_explicit_source_still_validates_source_kind() {
        let mut graph = ModelGraph::new();

        // Owner is valid for many relationship checks; explicit source is what should be validated.
        let owner_feature = Element::new_with_kind(ElementKind::Feature).with_name("OwnerFeature");
        let owner_id = graph.add_element(owner_feature);

        // Package is not a Feature, so this should fail source-kind validation.
        let wrong_source = Element::new_with_kind(ElementKind::Package).with_name("NotAFeature");
        let wrong_source_id = graph.add_element(wrong_source);

        let succession = Element::new_with_kind(ElementKind::SuccessionAsUsage)
            .with_owner(owner_id)
            .with_prop("source", Value::Ref(wrong_source_id));
        graph.add_element(succession);

        let errors = graph.validate_relationship_types();
        let source_errors: Vec<_> = errors
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    StructuralError::RelationshipSourceTypeMismatch {
                        relationship_kind: ElementKind::SuccessionAsUsage,
                        ..
                    }
                )
            })
            .collect();
        assert!(
            !source_errors.is_empty(),
            "Explicit source on SuccessionAsUsage should still be source-kind validated"
        );
    }

    // === Diagnostic Conversion Tests (Phase 5) ===

    #[test]
    fn structural_error_to_diagnostic() {
        use sysml_span::Diagnostic;

        let error = StructuralError::OrphanElement {
            element_id: ElementId::new_v4(),
            element_name: Some("TestElement".to_string()),
            element_kind: ElementKind::PartUsage,
        };

        let diag: Diagnostic = error.into();
        assert!(diag.is_error());
        assert_eq!(diag.code, Some("E001".to_string()));
        assert!(diag.message.contains("must be inside a namespace"));
        assert!(diag.message.contains("TestElement"));
    }

    #[test]
    fn all_structural_errors_have_codes() {
        use sysml_span::Diagnostic;

        let errors = vec![
            StructuralError::OrphanElement {
                element_id: ElementId::new_v4(),
                element_name: None,
                element_kind: ElementKind::PartUsage,
            },
            StructuralError::OwnershipCycle {
                element_ids: vec![ElementId::new_v4()],
            },
            StructuralError::DanglingMembershipRef {
                membership_id: ElementId::new_v4(),
                property: "test".to_string(),
                missing_id: ElementId::new_v4(),
            },
            StructuralError::RelationshipSourceTypeMismatch {
                relationship_id: ElementId::new_v4(),
                relationship_kind: ElementKind::FeatureTyping,
                source_id: ElementId::new_v4(),
                source_kind: ElementKind::Package,
                expected_kind: ElementKind::Feature,
            },
            StructuralError::RelationshipTargetTypeMismatch {
                relationship_id: ElementId::new_v4(),
                relationship_kind: ElementKind::FeatureTyping,
                target_id: ElementId::new_v4(),
                target_kind: ElementKind::Package,
                expected_kind: ElementKind::Type,
            },
            StructuralError::DanglingRelationshipRef {
                relationship_id: ElementId::new_v4(),
                endpoint: "source".to_string(),
                missing_id: ElementId::new_v4(),
            },
            StructuralError::DanglingOwningMembership {
                element_id: ElementId::new_v4(),
                element_name: None,
                missing_membership_id: ElementId::new_v4(),
            },
            StructuralError::InvalidOwningMembership {
                element_id: ElementId::new_v4(),
                membership_id: ElementId::new_v4(),
                membership_kind: ElementKind::Package,
            },
        ];

        let expected_codes = [
            "E001", "E002", "E003", "E004", "E005", "E006", "E007", "E008",
        ];

        for (error, expected_code) in errors.into_iter().zip(expected_codes.iter()) {
            let diag: Diagnostic = error.into();
            assert_eq!(
                diag.code,
                Some(expected_code.to_string()),
                "Wrong code for error"
            );
            assert!(diag.is_error());
        }
    }
}
