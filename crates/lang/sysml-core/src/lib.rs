//! # sysml-core
//!
//! Core model types for SysML v2: Element, Relationship, and ModelGraph.
//!
//! This crate provides the fundamental data structures for representing
//! SysML v2 models in memory.
//!
//! ## Features
//!
//! - `serde`: Enable serde serialization support
//!
//! ## ElementKind
//!
//! The `ElementKind` enum is generated at build time from the official SysML v2
//! specification TTL vocabulary files. It contains all 266 element types defined
//! in the KerML and SysML specifications.
//!
//! ## Typed Property Accessors
//!
//! This crate also provides typed property accessors generated from OSLC shapes.
//! Use `element.as_part_usage()` to get a typed accessor for PartUsage properties.

pub use sysml_id::{CanonicalKey, ElementId, QualifiedName};
pub use sysml_span::Span;

// Metadata types (formerly sysml-meta crate)
pub mod meta;
pub use meta::Value;

mod validation;
pub use validation::{
    validate_graph_properties, SemanticError, SemanticErrorKind, ValidationError,
    ValidationErrorKind, ValidationResult,
};

// Semantic validation checks (hand-implemented check functions)
pub mod semantic_checks;

// Error code registry
pub mod error_codes;

// Membership-based ownership modules (SysML v2 compliant)
mod factory;
mod membership;
mod namespace;
mod ownership;
mod structural_validation;

// Name resolution module (Phase 2d)
pub mod resolution;

// Query functions (formerly sysml-query crate)
pub mod query;

/// Element-level diff between two model graphs (snapshot/baseline comparison).
pub mod diff;

// Metadata query functions (ToolExecution / ToolVariable extraction)
pub mod metadata;

// Model elaboration pass (bridges parser output to execution crates)
pub mod elaborate;

pub mod expression_pretty;
pub mod member_print;

// Element ordering utilities (shared by health-check passes)
pub mod element_ordering;

// Canonical JSON serialization (formerly sysml-json crate)
#[cfg(feature = "serde")]
pub mod json;

// Physics-aware static analysis (ISQ classification, domain mapping, diagnostics)
pub mod physics;

// Occurrence model types (Life, Snapshot, OccurrenceInstance, OccurrenceRegistry)
pub mod occurrence;

// Spatial frame and coordinate transformation types (SpatialFrame, CoordinateTransformation, FrameRegistry)
pub mod spatial;

// View filter (spec ElementFilterMembership / viewCondition mechanism)
pub mod view_filter;
pub use view_filter::{FilterCombine, ViewFilter};

// View discovery (user-authored ViewUsage / ViewDefinition listing)
pub mod view_index;
pub use view_index::{
    build_view_index, viewpoints_by_stakeholder, views_by_viewpoint, views_create_scratch_snippet,
    ExposeRef, RenderingRef, ViewSummary,
};

// Import health diagnostics (model-level import checks)
mod import_health;
pub use import_health::import_health_diagnostics;
pub use import_health::import_health_diagnostics_with_context;

pub use factory::ElementFactory;
pub use membership::{MembershipBuilder, MembershipView, OwningMembershipView};
pub use structural_validation::StructuralError;

// Generated code: suppress clippy lints that cannot be fixed in source
mod generated_element_kind {
    #![allow(
        clippy::derivable_impls,
        clippy::needless_pass_by_value,
        clippy::str_to_string,
        clippy::unnecessary_map_or,
        clippy::redundant_closure,
        clippy::from_str_radix_10,
        clippy::manual_is_variant_and,
        clippy::unnecessary_get_then_check,
        clippy::should_implement_trait,
        clippy::match_like_matches_macro
    )]
    include!(concat!(env!("OUT_DIR"), "/element_kind.generated.rs"));
}
pub use generated_element_kind::*;

mod generated_enums {
    #![allow(
        clippy::derivable_impls,
        clippy::str_to_string,
        clippy::from_str_radix_10,
        clippy::manual_is_variant_and,
        clippy::unnecessary_map_or,
        clippy::should_implement_trait
    )]
    include!(concat!(env!("OUT_DIR"), "/enums.generated.rs"));
}
pub use generated_enums::*;

mod generated_properties {
    #![allow(
        clippy::str_to_string,
        clippy::unnecessary_get_then_check,
        clippy::needless_pass_by_value,
        clippy::redundant_closure,
        clippy::unnecessary_map_or,
        clippy::manual_is_variant_and,
        clippy::wildcard_imports
    )]
    use super::*;
    include!(concat!(env!("OUT_DIR"), "/properties.generated.rs"));
}
pub use generated_properties::*;

mod generated_semantic_validation {
    #![allow(
        clippy::str_to_string,
        clippy::needless_pass_by_value,
        clippy::unnecessary_map_or,
        clippy::manual_is_variant_and,
        clippy::wildcard_imports
    )]
    use super::*;
    include!(concat!(
        env!("OUT_DIR"),
        "/semantic_validation.generated.rs"
    ));
}
pub use generated_semantic_validation::*;

/// Cross-reference registry generated from Xtext grammar.
///
/// This module contains metadata about all cross-reference properties
/// including their target types and scoping strategies.
#[allow(
    clippy::str_to_string,
    clippy::unnecessary_map_or,
    clippy::manual_is_variant_and,
    clippy::indexing_slicing
)]
pub mod crossrefs {
    include!(concat!(env!("OUT_DIR"), "/crossrefs.generated.rs"));
}

mod element;
mod graph;
mod relationship;

pub use element::Element;
pub use element::{
    is_analysis_case_kind, is_package_kind, is_requirement_kind, is_verification_case_kind,
};
pub use expression_pretty::is_expression_kind;
pub use graph::ModelGraph;
pub use relationship::{Relationship, RelationshipKind};

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_graph() -> ModelGraph {
        let mut graph = ModelGraph::new();

        // Create a package
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPackage");
        let pkg_id = graph.add_element(pkg);

        // Create a part usage owned by the package
        let part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("TestPart")
            .with_owner(pkg_id.clone());
        let part_id = graph.add_element(part);

        // Create a requirement usage
        let req = Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("TestReq")
            .with_owner(pkg_id.clone());
        let req_id = graph.add_element(req);

        // Create a satisfy relationship
        let satisfy = Relationship::new(RelationshipKind::Satisfy, part_id, req_id);
        graph.add_relationship(satisfy);

        graph
    }

    #[test]
    fn add_and_get_element() {
        let mut graph = ModelGraph::new();
        let element = Element::new_with_kind(ElementKind::PartUsage).with_name("MyPart");
        let id = element.id.clone();
        graph.add_element(element);

        let retrieved = graph.get_element(&id).unwrap();
        assert_eq!(retrieved.name, Some("MyPart".to_string()));
    }

    #[test]
    fn element_new_with_key_uses_canonical_key_hash() {
        let project = CanonicalKey::root("p");
        let pkg_key = CanonicalKey::for_named(&project, "Package", "Foo");
        let elem = Element::new_with_key(ElementKind::Package, &pkg_key);

        // The element's id is exactly the canonical-key hash.
        assert_eq!(elem.id, pkg_key.to_element_id());
        assert_eq!(elem.kind, ElementKind::Package);
    }

    #[test]
    fn element_new_with_key_stable_across_calls() {
        let project = CanonicalKey::root("p");
        let key = CanonicalKey::for_named(&project, "PartUsage", "Bar");

        let a = Element::new_with_key(ElementKind::PartUsage, &key);
        let b = Element::new_with_key(ElementKind::PartUsage, &key);

        // Two calls with the same key yield equal ids.
        assert_eq!(a.id, b.id);
        // And nothing else accidentally diverges from the new_with_kind shape.
        assert_eq!(a.name, None);
        assert!(a.props.is_empty());
        assert!(a.spans.is_empty());
    }

    #[test]
    fn element_new_with_key_distinct_keys_distinct_ids() {
        let parent = CanonicalKey::root("p");
        let foo = CanonicalKey::for_named(&parent, "Package", "Foo");
        let bar = CanonicalKey::for_named(&parent, "Package", "Bar");

        let a = Element::new_with_key(ElementKind::Package, &foo);
        let b = Element::new_with_key(ElementKind::Package, &bar);

        assert_ne!(a.id, b.id);
    }

    #[test]
    fn relationship_new_with_key_uses_canonical_key_hash() {
        let project = CanonicalKey::root("p");
        let edge_key = CanonicalKey::for_anonymous(&project, "Specialize:source", 0);
        let src = ElementId::new_v4();
        let dst = ElementId::new_v4();

        let rel = Relationship::new_with_key(
            RelationshipKind::Specialize,
            src.clone(),
            dst.clone(),
            &edge_key,
        );

        assert_eq!(rel.id, edge_key.to_element_id());
        assert_eq!(rel.kind, RelationshipKind::Specialize);
        assert_eq!(rel.source, src);
        assert_eq!(rel.target, dst);
    }

    #[test]
    fn relationship_new_with_key_stable_across_calls() {
        let project = CanonicalKey::root("p");
        let edge_key = CanonicalKey::for_anonymous(&project, "Satisfy:membership", 3);
        let src = ElementId::new_v4();
        let dst = ElementId::new_v4();

        let a = Relationship::new_with_key(
            RelationshipKind::Satisfy,
            src.clone(),
            dst.clone(),
            &edge_key,
        );
        let b = Relationship::new_with_key(RelationshipKind::Satisfy, src, dst, &edge_key);

        assert_eq!(a.id, b.id);
    }

    #[test]
    fn add_and_get_relationship() {
        let mut graph = ModelGraph::new();
        let e1 = Element::new_with_kind(ElementKind::PartUsage);
        let e2 = Element::new_with_kind(ElementKind::RequirementUsage);
        let id1 = graph.add_element(e1);
        let id2 = graph.add_element(e2);

        let rel = Relationship::new(RelationshipKind::Satisfy, id1.clone(), id2.clone());
        let rel_id = rel.id.clone();
        graph.add_relationship(rel);

        let retrieved = graph.get_relationship(&rel_id).unwrap();
        assert_eq!(retrieved.source, id1);
        assert_eq!(retrieved.target, id2);
    }

    #[test]
    fn children_of() {
        let graph = create_test_graph();
        let pkg = graph
            .elements_by_kind(&ElementKind::Package)
            .next()
            .unwrap();
        let children: Vec<_> = graph.children_of(&pkg.id).collect();
        assert_eq!(children.len(), 2); // PartUsage and RequirementUsage
    }

    #[test]
    fn outgoing_relationships() {
        let graph = create_test_graph();
        let part = graph
            .elements_by_kind(&ElementKind::PartUsage)
            .next()
            .unwrap();
        let outgoing: Vec<_> = graph.outgoing(&part.id).collect();
        assert_eq!(outgoing.len(), 1);
        assert!(matches!(outgoing[0].kind, RelationshipKind::Satisfy));
    }

    #[test]
    fn elements_by_kind() {
        let graph = create_test_graph();
        let packages: Vec<_> = graph.elements_by_kind(&ElementKind::Package).collect();
        assert_eq!(packages.len(), 1);

        let parts: Vec<_> = graph.elements_by_kind(&ElementKind::PartUsage).collect();
        assert_eq!(parts.len(), 1);
    }

    #[test]
    fn roots() {
        let graph = create_test_graph();
        let roots: Vec<_> = graph.roots().collect();
        assert_eq!(roots.len(), 1);
        assert!(matches!(roots[0].kind, ElementKind::Package));
    }

    #[test]
    fn element_with_props() {
        let element = Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("Req1")
            .with_prop("priority", 1i64)
            .with_prop("verified", false);

        assert_eq!(
            element.get_prop("priority").and_then(|v| v.as_int()),
            Some(1)
        );
        assert_eq!(
            element.get_prop("verified").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn graph_counts() {
        let graph = create_test_graph();
        assert_eq!(graph.element_count(), 3);
        assert_eq!(graph.relationship_count(), 1);
        assert!(!graph.is_empty());
    }

    #[test]
    fn element_kind_from_str() {
        assert_eq!(ElementKind::from_str("Package"), Some(ElementKind::Package));
        assert_eq!(
            ElementKind::from_str("PartUsage"),
            Some(ElementKind::PartUsage)
        );
        assert_eq!(ElementKind::from_str("InvalidType"), None);
    }

    #[test]
    fn element_kind_has_all_types() {
        // Verify the enum has the expected number of types
        // The count is the unique types after deduplication between KerML and SysML
        let count = ElementKind::count();
        // At least 150 unique types (some are duplicated between KerML and SysML)
        assert!(count >= 150, "Expected at least 150 types, got {}", count);
        assert!(count <= 300, "Expected at most 300 types, got {}", count);
    }

    #[test]
    fn element_kind_iter() {
        let count = ElementKind::iter().count();
        assert_eq!(count, ElementKind::count());
    }

    #[test]
    fn typed_property_accessor_cast() {
        // Test that we can cast an element to a typed accessor
        let element = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("TestPart")
            .with_prop("isVariation", false)
            .with_prop("isComposite", true);

        // Cast to PartUsageProps
        let part_props = element.as_part_usage();
        assert!(part_props.is_some());
        let part = part_props.unwrap();

        // Access underlying element
        assert_eq!(part.element().name, Some("TestPart".to_string()));
        assert_eq!(part.element().kind, ElementKind::PartUsage);
    }

    #[test]
    fn typed_property_accessor_wrong_kind() {
        // Test that casting fails for wrong element kind
        let element = Element::new_with_kind(ElementKind::Package);

        // Should not cast to PartUsageProps
        assert!(element.as_part_usage().is_none());
        // Should cast to PackageProps
        assert!(element.as_package().is_some());
    }

    #[test]
    fn property_accessor_validation() {
        // Test validation on a typed accessor
        let element = Element::new_with_kind(ElementKind::PartUsage);

        let part = element.as_part_usage().unwrap();
        let result = part.validate();

        // Validation runs without panicking
        // There may be missing required properties, but the point is it doesn't panic
        let _ = result.error_count();
    }

    // === Tests for Phase 0c: Type Hierarchy & Enumerations ===

    #[test]
    fn test_supertypes() {
        let supertypes = ElementKind::PartUsage.supertypes();
        assert!(supertypes.contains(&ElementKind::ItemUsage));
        assert!(supertypes.contains(&ElementKind::Usage));
        assert!(supertypes.contains(&ElementKind::Feature));
        assert!(supertypes.contains(&ElementKind::Type));
        assert!(supertypes.contains(&ElementKind::Element));
        // Should not contain itself or unrelated types
        assert!(!supertypes.contains(&ElementKind::PartUsage));
        assert!(!supertypes.contains(&ElementKind::Relationship));
    }

    #[test]
    fn test_direct_supertypes() {
        // PartUsage's direct supertype should be ItemUsage
        let direct = ElementKind::PartUsage.direct_supertypes();
        assert!(direct.contains(&ElementKind::ItemUsage));
        // Should not include transitive supertypes
        assert!(!direct.contains(&ElementKind::Element));
    }

    #[test]
    fn test_is_subtype_of() {
        assert!(ElementKind::PartUsage.is_subtype_of(ElementKind::Feature));
        assert!(ElementKind::PartUsage.is_subtype_of(ElementKind::Element));
        assert!(ElementKind::Feature.is_subtype_of(ElementKind::Type));
        // A type is not a subtype of itself
        assert!(!ElementKind::Feature.is_subtype_of(ElementKind::Feature));
        // Element is the root, not a subtype of anything
        assert!(!ElementKind::Element.is_subtype_of(ElementKind::Feature));
    }

    #[test]
    fn test_is_definition_predicate() {
        assert!(ElementKind::PartDefinition.is_definition());
        assert!(ElementKind::ActionDefinition.is_definition());
        assert!(!ElementKind::PartUsage.is_definition());
        assert!(!ElementKind::Element.is_definition());
    }

    #[test]
    fn test_is_usage_predicate() {
        assert!(ElementKind::PartUsage.is_usage());
        assert!(ElementKind::ActionUsage.is_usage());
        assert!(!ElementKind::PartDefinition.is_usage());
        assert!(!ElementKind::Element.is_usage());
    }

    #[test]
    fn test_is_relationship_predicate() {
        assert!(ElementKind::Relationship.is_relationship());
        assert!(ElementKind::Specialization.is_relationship());
        assert!(ElementKind::FeatureTyping.is_relationship());
        assert!(!ElementKind::Element.is_relationship());
        assert!(!ElementKind::PartUsage.is_relationship());
    }

    #[test]
    fn test_is_classifier_predicate() {
        assert!(ElementKind::Classifier.is_classifier());
        assert!(ElementKind::Class.is_classifier());
        assert!(!ElementKind::Element.is_classifier());
        assert!(!ElementKind::Feature.is_classifier());
    }

    #[test]
    fn test_is_feature_predicate() {
        assert!(ElementKind::Feature.is_feature());
        assert!(ElementKind::PartUsage.is_feature());
        assert!(ElementKind::Connector.is_feature());
        assert!(!ElementKind::Element.is_feature());
        assert!(!ElementKind::Relationship.is_feature());
    }

    #[test]
    fn test_corresponding_usage() {
        assert_eq!(
            ElementKind::PartDefinition.corresponding_usage(),
            Some(ElementKind::PartUsage)
        );
        assert_eq!(
            ElementKind::ActionDefinition.corresponding_usage(),
            Some(ElementKind::ActionUsage)
        );
        assert_eq!(ElementKind::Element.corresponding_usage(), None);
        assert_eq!(ElementKind::PartUsage.corresponding_usage(), None);
    }

    #[test]
    fn test_corresponding_definition() {
        assert_eq!(
            ElementKind::PartUsage.corresponding_definition(),
            Some(ElementKind::PartDefinition)
        );
        assert_eq!(
            ElementKind::ActionUsage.corresponding_definition(),
            Some(ElementKind::ActionDefinition)
        );
        assert_eq!(ElementKind::Element.corresponding_definition(), None);
        assert_eq!(ElementKind::PartDefinition.corresponding_definition(), None);
    }

    #[test]
    fn test_relationship_source_type() {
        assert_eq!(
            ElementKind::FeatureTyping.relationship_source_type(),
            Some(ElementKind::Feature)
        );
        assert_eq!(
            ElementKind::Specialization.relationship_source_type(),
            Some(ElementKind::Type)
        );
        assert_eq!(
            ElementKind::Relationship.relationship_source_type(),
            Some(ElementKind::Element)
        );
        // Non-relationships return None
        assert_eq!(ElementKind::Element.relationship_source_type(), None);
        assert_eq!(ElementKind::PartUsage.relationship_source_type(), None);
    }

    #[test]
    fn test_relationship_target_type() {
        assert_eq!(
            ElementKind::FeatureTyping.relationship_target_type(),
            Some(ElementKind::Type)
        );
        assert_eq!(
            ElementKind::Subsetting.relationship_target_type(),
            Some(ElementKind::Feature)
        );
        // Non-relationships return None
        assert_eq!(ElementKind::Element.relationship_target_type(), None);
    }

    #[test]
    fn test_feature_direction_kind() {
        assert_eq!(FeatureDirectionKind::In.as_str(), "in");
        assert_eq!(FeatureDirectionKind::Out.as_str(), "out");
        assert_eq!(FeatureDirectionKind::Inout.as_str(), "inout");

        assert_eq!(
            FeatureDirectionKind::from_str("in"),
            Some(FeatureDirectionKind::In)
        );
        assert_eq!(
            FeatureDirectionKind::from_str("out"),
            Some(FeatureDirectionKind::Out)
        );
        assert_eq!(FeatureDirectionKind::from_str("invalid"), None);

        assert_eq!(FeatureDirectionKind::count(), 3);
        assert_eq!(FeatureDirectionKind::iter().count(), 3);
    }

    #[test]
    fn test_visibility_kind() {
        assert_eq!(VisibilityKind::Public.as_str(), "public");
        assert_eq!(VisibilityKind::Private.as_str(), "private");
        assert_eq!(VisibilityKind::Protected.as_str(), "protected");

        assert_eq!(
            VisibilityKind::from_str("public"),
            Some(VisibilityKind::Public)
        );
        assert_eq!(VisibilityKind::count(), 3);
    }

    #[test]
    fn test_state_subaction_kind() {
        // "do" is a reserved keyword, so the variant is Do_
        assert_eq!(StateSubactionKind::Entry.as_str(), "entry");
        assert_eq!(StateSubactionKind::Do_.as_str(), "do");
        assert_eq!(StateSubactionKind::Exit.as_str(), "exit");

        assert_eq!(
            StateSubactionKind::from_str("do"),
            Some(StateSubactionKind::Do_)
        );
        assert_eq!(StateSubactionKind::count(), 3);
    }

    #[test]
    fn test_all_value_enums_exist() {
        // Verify all 7 value enums are available
        let _ = FeatureDirectionKind::default();
        let _ = VisibilityKind::default();
        let _ = PortionKind::default();
        let _ = RequirementConstraintKind::default();
        let _ = StateSubactionKind::default();
        let _ = TransitionFeatureKind::default();
        let _ = TriggerKind::default();
    }

    // === Tests for Phase 2e: Library Index Merge Fix ===

    #[test]
    fn merge_preserves_namespace_to_memberships() {
        let mut graph1 = ModelGraph::new();
        let mut graph2 = ModelGraph::new();

        // Create a package in graph2
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPkg");
        let pkg_id = graph2.add_element(pkg);

        // Create a member in graph2 with ownership via add_owned_element
        let member = Element::new_with_kind(ElementKind::PartUsage).with_name("TestMember");
        let _member_id = graph2.add_owned_element(member, pkg_id.clone(), VisibilityKind::Public);

        // Verify graph2 has the namespace_to_memberships index entry
        assert!(
            graph2.namespace_to_memberships.contains_key(&pkg_id),
            "graph2 should have namespace_to_memberships entry for the package"
        );

        // Merge graph2 into graph1
        graph1.merge(graph2, false);

        // Verify graph1 now has the index entry (critical for library resolution)
        assert!(
            graph1.namespace_to_memberships.contains_key(&pkg_id),
            "After merge, graph1 should have namespace_to_memberships entry from graph2"
        );
    }

    #[test]
    fn merge_preserves_owner_to_children() {
        let mut graph1 = ModelGraph::new();
        let mut graph2 = ModelGraph::new();

        // Create a package with a child in graph2
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Parent");
        let pkg_id = graph2.add_element(pkg);

        let child = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("Child")
            .with_owner(pkg_id.clone());
        let child_id = graph2.add_element(child);

        // Verify graph2 has the owner_to_children index
        assert!(graph2
            .owner_to_children
            .get(&pkg_id)
            .map_or(false, |children| children.contains(&child_id)));

        // Merge and verify
        graph1.merge(graph2, false);
        assert!(
            graph1
                .owner_to_children
                .get(&pkg_id)
                .map_or(false, |children| children.contains(&child_id)),
            "owner_to_children should be preserved after merge"
        );
    }

    #[test]
    fn merge_as_library_registers_root_packages() {
        let mut graph1 = ModelGraph::new();
        let mut graph2 = ModelGraph::new();

        // Create a root package in graph2
        let lib_pkg = Element::new_with_kind(ElementKind::Package).with_name("LibraryPkg");
        let lib_pkg_id = graph2.add_element(lib_pkg);

        // Merge as library
        graph1.merge(graph2, true);

        // Verify the package is registered as a library package
        assert!(
            graph1.is_library_package(&lib_pkg_id),
            "Root packages should be registered as library packages when as_library=true"
        );
    }

    // === Tests for the lazy fingerprint cache (WS2 perf) ===

    #[test]
    fn fingerprint_is_stable_across_repeated_calls() {
        let mut graph = ModelGraph::new();
        graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("P"));
        let first = graph.fingerprint();
        // Repeated calls (cache hits) must return the identical value.
        assert_eq!(first, graph.fingerprint());
        assert_eq!(first, graph.fingerprint());
    }

    #[test]
    fn fingerprint_changes_when_element_added() {
        let mut graph = ModelGraph::new();
        graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("A"));
        let before = graph.fingerprint(); // populates cache
        graph.add_element(Element::new_with_kind(ElementKind::PartUsage).with_name("B"));
        let after = graph.fingerprint(); // cache must have been invalidated
        assert_ne!(
            before, after,
            "adding an element must invalidate the fingerprint cache"
        );
    }

    #[test]
    fn fingerprint_changes_when_element_renamed_via_get_mut() {
        let mut graph = ModelGraph::new();
        let id = graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("Old"));
        let before = graph.fingerprint(); // populates cache
        if let Some(e) = graph.get_element_mut(&id) {
            e.name = Some("New".to_string());
        }
        let after = graph.fingerprint();
        assert_ne!(
            before, after,
            "renaming via get_element_mut must invalidate the fingerprint cache"
        );
    }

    // === Tests for the relationship_kind_index (WS2 perf) ===

    #[test]
    fn relationships_by_kind_uses_index_and_matches_scan() {
        use crate::relationship::{Relationship, RelationshipKind};
        let mut graph = ModelGraph::new();
        let a = graph.add_element(Element::new_with_kind(ElementKind::PartUsage).with_name("A"));
        let b = graph.add_element(Element::new_with_kind(ElementKind::PartUsage).with_name("B"));
        let c = graph.add_element(Element::new_with_kind(ElementKind::PartUsage).with_name("C"));
        graph.add_relationship(Relationship::new(
            RelationshipKind::Specialize,
            a.clone(),
            b.clone(),
        ));
        graph.add_relationship(Relationship::new(RelationshipKind::Reference, a.clone(), c));
        graph.add_relationship(Relationship::new(RelationshipKind::Specialize, b, a));

        let spec: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Specialize)
            .collect();
        assert_eq!(spec.len(), 2, "index must return both Specialize rels once");
        assert!(spec.iter().all(|r| r.kind == RelationshipKind::Specialize));
        assert_eq!(
            graph
                .relationships_by_kind(&RelationshipKind::Reference)
                .count(),
            1
        );
        assert_eq!(
            graph
                .relationships_by_kind(&RelationshipKind::Transition)
                .count(),
            0,
            "absent kind yields nothing"
        );
    }

    #[test]
    fn relationship_kind_index_survives_rebuild() {
        use crate::relationship::{Relationship, RelationshipKind};
        let mut graph = ModelGraph::new();
        let a = graph.add_element(Element::new_with_kind(ElementKind::PartUsage).with_name("A"));
        let b = graph.add_element(Element::new_with_kind(ElementKind::PartUsage).with_name("B"));
        graph.add_relationship(Relationship::new(
            RelationshipKind::Specialize,
            a.clone(),
            b,
        ));
        let before = graph
            .relationships_by_kind(&RelationshipKind::Specialize)
            .count();
        graph.rebuild_indexes();
        let after = graph
            .relationships_by_kind(&RelationshipKind::Specialize)
            .count();
        assert_eq!(before, after, "rebuild_indexes must repopulate the index");
        assert_eq!(after, 1);
    }

    #[test]
    fn fingerprint_matches_uncached_recompute() {
        // The cached value must equal a fresh structural recompute. Build a
        // graph, snapshot the (cached) fingerprint, then clone (clone resets
        // the cache) and recompute on the clone — they must agree.
        let mut graph = ModelGraph::new();
        let pkg = graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("Pkg"));
        graph.add_element(
            Element::new_with_kind(ElementKind::PartUsage)
                .with_name("Part")
                .with_owner(pkg),
        );
        let cached = graph.fingerprint();
        let clone = graph.clone();
        assert_eq!(
            cached,
            clone.fingerprint(),
            "cached fingerprint must equal a fresh recompute on an identical graph"
        );
    }

    // =========================================================================
    // effectiveName() / effectiveShortName() tests
    // Spec: Kerml-Vocab.ttl - name/shortName are derived properties
    // =========================================================================

    #[test]
    fn effective_name_returns_declared_name() {
        let graph = ModelGraph::new();
        let element = Element::new_with_kind(ElementKind::PartUsage).with_name("MyPart");

        assert_eq!(
            element.effective_name(&graph),
            Some("MyPart"),
            "effective_name should return the declared name when set"
        );
    }

    #[test]
    fn effective_name_returns_none_without_name_or_membership() {
        let graph = ModelGraph::new();
        let element = Element::new_with_kind(ElementKind::PartUsage);

        assert_eq!(
            element.effective_name(&graph),
            None,
            "effective_name should return None when no name or membership"
        );
    }

    #[test]
    fn effective_name_falls_back_to_membership_member_name() {
        let mut graph = ModelGraph::new();

        // Create a package (owner)
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        // Create an element WITHOUT a declared name
        let elem = Element::new_with_kind(ElementKind::PartUsage);
        let elem_id = elem.id.clone();

        // Add it as owned element with a specific member name via membership
        graph.add_owned_element(elem, pkg_id.clone(), VisibilityKind::Public);

        // Now manually set the memberName on the owning membership
        // Find the membership element
        let memberships: Vec<_> = graph
            .namespace_to_memberships
            .get(&pkg_id)
            .map(|s| s.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        assert!(
            !memberships.is_empty(),
            "Should have an owning membership for the element"
        );

        // Set memberName on the membership
        if let Some(membership) = graph.elements.get_mut(&memberships[0]) {
            membership.set_prop(
                membership::props::MEMBER_NAME,
                Value::String("AliasName".to_string()),
            );
        }

        // Now check effective_name on the owned element
        let elem = graph.get_element(&elem_id).unwrap();
        assert_eq!(
            elem.effective_name(&graph),
            Some("AliasName"),
            "effective_name should fall back to membership memberName"
        );
    }

    #[test]
    fn effective_name_prefers_declared_over_membership() {
        let mut graph = ModelGraph::new();

        // Create a package (owner)
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        // Create an element WITH a declared name
        let elem = Element::new_with_kind(ElementKind::PartUsage).with_name("DeclaredName");
        let elem_id = elem.id.clone();

        graph.add_owned_element(elem, pkg_id.clone(), VisibilityKind::Public);

        // Set a different memberName on the membership
        let memberships: Vec<_> = graph
            .namespace_to_memberships
            .get(&pkg_id)
            .map(|s| s.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        if let Some(membership) = graph.elements.get_mut(&memberships[0]) {
            membership.set_prop(
                membership::props::MEMBER_NAME,
                Value::String("MemberAlias".to_string()),
            );
        }

        // Declared name should win
        let elem = graph.get_element(&elem_id).unwrap();
        assert_eq!(
            elem.effective_name(&graph),
            Some("DeclaredName"),
            "effective_name should prefer declared name over membership memberName"
        );
    }

    #[test]
    fn effective_short_name_returns_declared_short_name() {
        let graph = ModelGraph::new();
        let element =
            Element::new_with_kind(ElementKind::PartUsage).with_prop("declaredShortName", "MP");

        assert_eq!(
            element.effective_short_name(&graph),
            Some("MP"),
            "effective_short_name should return declaredShortName when set"
        );
    }

    #[test]
    fn effective_short_name_returns_none_without_short_name_or_membership() {
        let graph = ModelGraph::new();
        let element = Element::new_with_kind(ElementKind::PartUsage);

        assert_eq!(
            element.effective_short_name(&graph),
            None,
            "effective_short_name should return None when no short name or membership"
        );
    }

    #[test]
    fn effective_short_name_falls_back_to_membership_member_short_name() {
        let mut graph = ModelGraph::new();

        // Create a package (owner)
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        // Create an element WITHOUT a declared short name
        let elem = Element::new_with_kind(ElementKind::PartUsage);
        let elem_id = elem.id.clone();

        graph.add_owned_element(elem, pkg_id.clone(), VisibilityKind::Public);

        // Set memberShortName on the membership
        let memberships: Vec<_> = graph
            .namespace_to_memberships
            .get(&pkg_id)
            .map(|s| s.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        if let Some(membership) = graph.elements.get_mut(&memberships[0]) {
            membership.set_prop(
                membership::props::MEMBER_SHORT_NAME,
                Value::String("SN".to_string()),
            );
        }

        let elem = graph.get_element(&elem_id).unwrap();
        assert_eq!(
            elem.effective_short_name(&graph),
            Some("SN"),
            "effective_short_name should fall back to membership memberShortName"
        );
    }

    #[test]
    fn effective_short_name_prefers_declared_over_membership() {
        let mut graph = ModelGraph::new();

        // Create a package (owner)
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        // Create an element WITH a declared short name
        let elem =
            Element::new_with_kind(ElementKind::PartUsage).with_prop("declaredShortName", "DSN");
        let elem_id = elem.id.clone();

        graph.add_owned_element(elem, pkg_id.clone(), VisibilityKind::Public);

        // Set a different memberShortName on the membership
        let memberships: Vec<_> = graph
            .namespace_to_memberships
            .get(&pkg_id)
            .map(|s| s.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        if let Some(membership) = graph.elements.get_mut(&memberships[0]) {
            membership.set_prop(
                membership::props::MEMBER_SHORT_NAME,
                Value::String("MSN".to_string()),
            );
        }

        let elem = graph.get_element(&elem_id).unwrap();
        assert_eq!(
            elem.effective_short_name(&graph),
            Some("DSN"),
            "effective_short_name should prefer declared short name over membership"
        );
    }
}
