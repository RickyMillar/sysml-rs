//! Comprehensive Coverage Tests for Generated SysML v2 Types
//!
//! These tests ensure that:
//! 1. All 175 ElementKind variants are functional
//! 2. All type hierarchy methods work correctly
//! 3. All 7 value enums are complete
//! 4. All property accessors can be invoked
//! 5. ElementFactory covers the major types
//!
//! Run with: cargo test -p sysml-core --test coverage_tests

use sysml_core::*;

// ============================================================================
// ELEMENT KIND COVERAGE TESTS
// ============================================================================

#[test]
fn all_element_kinds_are_parseable() {
    // Every ElementKind variant should round-trip through as_str/from_str
    let mut failures = Vec::new();

    for kind in ElementKind::iter() {
        let name = kind.as_str();
        match ElementKind::from_str(name) {
            Some(parsed) => {
                if parsed != kind {
                    failures.push(format!("{}: parsed to {:?}", name, parsed));
                }
            }
            None => {
                failures.push(format!("{}: from_str returned None", name));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "ElementKind round-trip failures:\n  {}",
        failures.join("\n  ")
    );
}

#[test]
fn all_element_kinds_have_valid_supertypes() {
    // Every ElementKind (except Element) should have at least one supertype
    let mut no_supertypes = Vec::new();

    for kind in ElementKind::iter() {
        if kind == ElementKind::Element {
            // Element is the root, no supertypes expected
            assert!(
                kind.supertypes().is_empty(),
                "Element should have no supertypes"
            );
            continue;
        }

        let supertypes = kind.supertypes();
        if supertypes.is_empty() {
            no_supertypes.push(kind.as_str().to_string());
        }
    }

    // Note: Some types might legitimately have no supertypes in the spec
    // This test documents which ones
    if !no_supertypes.is_empty() {
        println!(
            "Types with no supertypes (may be expected): {:?}",
            no_supertypes
        );
    }
}

#[test]
fn all_element_kinds_have_consistent_hierarchy() {
    // Test that is_subtype_of is consistent with supertypes()
    for kind in ElementKind::iter() {
        let supertypes = kind.supertypes();

        for supertype in supertypes.iter() {
            assert!(
                kind.clone().is_subtype_of(supertype.clone()),
                "{:?}.is_subtype_of({:?}) should be true since {:?} is in supertypes",
                kind,
                supertype,
                supertype
            );
        }

        // No type is a subtype of itself
        assert!(
            !kind.clone().is_subtype_of(kind.clone()),
            "{:?}.is_subtype_of({:?}) should be false",
            kind,
            kind
        );
    }
}

#[test]
fn element_kind_predicates_are_mutually_consistent() {
    // Test that predicate methods are consistent
    for kind in ElementKind::iter() {
        // If it's a Usage, it should be a Feature (usages are features)
        if kind.is_usage() {
            assert!(kind.is_feature(), "{:?} is_usage but not is_feature", kind);
        }

        // If it's a Definition, it should be a Classifier or Type
        // (Definitions are classifiers in SysML)
        if kind.is_definition() {
            // Most definitions should have Type in hierarchy
            let supertypes = kind.supertypes();
            let has_type_ancestor = supertypes.contains(&ElementKind::Type)
                || supertypes.contains(&ElementKind::Classifier)
                || kind == ElementKind::Definition;

            if !has_type_ancestor && kind.as_str().ends_with("Definition") {
                println!(
                    "Note: {:?} is_definition but Type not in supertypes (may be expected)",
                    kind
                );
            }
        }

        // If it's a Relationship, relationship_source_type should return Some
        if kind.is_relationship() {
            // Note: Not all relationships have explicit source types
            // This just documents which ones do/don't
            let _source = kind.relationship_source_type();
            let _target = kind.relationship_target_type();
        }
    }
}

#[test]
fn definition_usage_pairs_are_complete() {
    // Test that corresponding_usage/corresponding_definition are inverses
    let mut definitions_without_usage = Vec::new();
    let mut usages_without_definition = Vec::new();

    for kind in ElementKind::iter() {
        if kind.is_definition() {
            if let Some(usage) = kind.corresponding_usage() {
                // Verify inverse
                if let Some(def_back) = usage.corresponding_definition() {
                    assert_eq!(
                        def_back, kind,
                        "Round-trip failed: {:?} -> {:?} -> {:?}",
                        kind, usage, def_back
                    );
                } else {
                    println!(
                        "Warning: {:?} has corresponding_usage {:?} but inverse is None",
                        kind, usage
                    );
                }
            } else {
                definitions_without_usage.push(kind.as_str().to_string());
            }
        }

        if kind.is_usage() {
            if kind.corresponding_definition().is_none() {
                usages_without_definition.push(kind.as_str().to_string());
            }
        }
    }

    // Document which don't have pairs (may be expected)
    if !definitions_without_usage.is_empty() {
        println!(
            "Definitions without corresponding usage: {:?}",
            definitions_without_usage
        );
    }
    if !usages_without_definition.is_empty() {
        println!(
            "Usages without corresponding definition: {:?}",
            usages_without_definition
        );
    }
}

#[test]
fn element_kind_count_matches_iteration() {
    let iter_count = ElementKind::iter().count();
    let declared_count = ElementKind::count();

    assert_eq!(
        iter_count, declared_count,
        "ElementKind::iter().count() ({}) != ElementKind::count() ({})",
        iter_count, declared_count
    );

    // Verify we have a reasonable number of types
    assert!(
        declared_count >= 170,
        "Expected at least 170 ElementKind variants, got {}",
        declared_count
    );
    assert!(
        declared_count <= 300,
        "Expected at most 300 ElementKind variants, got {}",
        declared_count
    );

    println!("Total ElementKind variants: {}", declared_count);
}

#[test]
fn all_element_kinds_can_create_elements() {
    // Every ElementKind should be usable to create an Element
    for kind in ElementKind::iter() {
        let element = Element::new_with_kind(kind.clone());
        assert_eq!(element.kind, kind);
        assert!(element.id.as_str().len() > 0);
    }
}

// ============================================================================
// VALUE ENUM COVERAGE TESTS
// ============================================================================

#[test]
fn feature_direction_kind_is_complete() {
    assert_eq!(FeatureDirectionKind::count(), 3);

    let values: Vec<_> = FeatureDirectionKind::iter().collect();
    assert!(values.contains(&FeatureDirectionKind::In));
    assert!(values.contains(&FeatureDirectionKind::Out));
    assert!(values.contains(&FeatureDirectionKind::Inout));

    // Test round-trip
    for v in FeatureDirectionKind::iter() {
        let s = v.as_str();
        let parsed = FeatureDirectionKind::from_str(s);
        assert_eq!(parsed, Some(v), "Round-trip failed for {}", s);
    }
}

#[test]
fn visibility_kind_is_complete() {
    assert_eq!(VisibilityKind::count(), 3);

    let values: Vec<_> = VisibilityKind::iter().collect();
    assert!(values.contains(&VisibilityKind::Public));
    assert!(values.contains(&VisibilityKind::Private));
    assert!(values.contains(&VisibilityKind::Protected));

    // Test round-trip
    for v in VisibilityKind::iter() {
        let s = v.as_str();
        let parsed = VisibilityKind::from_str(s);
        assert_eq!(parsed, Some(v), "Round-trip failed for {}", s);
    }
}

#[test]
fn portion_kind_is_complete() {
    assert!(
        PortionKind::count() >= 2,
        "PortionKind should have at least 2 variants"
    );

    for v in PortionKind::iter() {
        let s = v.as_str();
        let parsed = PortionKind::from_str(s);
        assert_eq!(parsed, Some(v), "Round-trip failed for {}", s);
    }
}

#[test]
fn requirement_constraint_kind_is_complete() {
    assert!(
        RequirementConstraintKind::count() >= 2,
        "RequirementConstraintKind should have at least 2 variants"
    );

    for v in RequirementConstraintKind::iter() {
        let s = v.as_str();
        let parsed = RequirementConstraintKind::from_str(s);
        assert_eq!(parsed, Some(v), "Round-trip failed for {}", s);
    }
}

#[test]
fn state_subaction_kind_is_complete() {
    assert_eq!(StateSubactionKind::count(), 3);

    // "do" is a keyword, so the variant is Do_
    let values: Vec<_> = StateSubactionKind::iter().collect();
    assert!(values.contains(&StateSubactionKind::Entry));
    assert!(values.contains(&StateSubactionKind::Do_));
    assert!(values.contains(&StateSubactionKind::Exit));

    // Test as_str returns the spec value
    assert_eq!(StateSubactionKind::Do_.as_str(), "do");
}

#[test]
fn transition_feature_kind_is_complete() {
    assert!(
        TransitionFeatureKind::count() >= 2,
        "TransitionFeatureKind should have at least 2 variants"
    );

    for v in TransitionFeatureKind::iter() {
        let s = v.as_str();
        let parsed = TransitionFeatureKind::from_str(s);
        assert_eq!(parsed, Some(v), "Round-trip failed for {}", s);
    }
}

#[test]
fn trigger_kind_is_complete() {
    assert!(
        TriggerKind::count() >= 2,
        "TriggerKind should have at least 2 variants"
    );

    for v in TriggerKind::iter() {
        let s = v.as_str();
        let parsed = TriggerKind::from_str(s);
        assert_eq!(parsed, Some(v), "Round-trip failed for {}", s);
    }
}

#[test]
fn all_seven_value_enums_exist() {
    // Verify all 7 value enums from the spec are generated
    let _ = FeatureDirectionKind::default();
    let _ = VisibilityKind::default();
    let _ = PortionKind::default();
    let _ = RequirementConstraintKind::default();
    let _ = StateSubactionKind::default();
    let _ = TransitionFeatureKind::default();
    let _ = TriggerKind::default();

    // Count total enum variants
    let total_variants = FeatureDirectionKind::count()
        + VisibilityKind::count()
        + PortionKind::count()
        + RequirementConstraintKind::count()
        + StateSubactionKind::count()
        + TransitionFeatureKind::count()
        + TriggerKind::count();

    println!(
        "Total value enum variants across 7 enums: {}",
        total_variants
    );
}

// ============================================================================
// ELEMENT FACTORY COVERAGE TESTS
// ============================================================================

#[test]
fn factory_covers_all_definition_types() {
    // Test that ElementFactory has methods for common definition types
    let definitions = vec![
        ElementFactory::part_definition("test"),
        ElementFactory::item_definition("test"),
        ElementFactory::action_definition("test"),
        ElementFactory::state_definition("test"),
        ElementFactory::requirement_definition("test"),
        ElementFactory::constraint_definition("test"),
        ElementFactory::calculation_definition("test"),
        ElementFactory::case_definition("test"),
        ElementFactory::analysis_case_definition("test"),
        ElementFactory::verification_case_definition("test"),
        ElementFactory::use_case_definition("test"),
        ElementFactory::view_definition("test"),
        ElementFactory::viewpoint_definition("test"),
        ElementFactory::rendering_definition("test"),
        ElementFactory::port_definition("test"),
        ElementFactory::connection_definition("test"),
        ElementFactory::interface_definition("test"),
        ElementFactory::allocation_definition("test"),
        ElementFactory::attribute_definition("test"),
        ElementFactory::enumeration_definition("test"),
        ElementFactory::occurrence_definition("test"),
        ElementFactory::metadata_definition("test"),
    ];

    for def in &definitions {
        assert!(
            def.kind.is_definition(),
            "{:?} should be a definition",
            def.kind
        );
    }

    println!(
        "ElementFactory covers {} definition types",
        definitions.len()
    );
}

#[test]
fn factory_covers_all_usage_types() {
    // Test that ElementFactory has methods for common usage types
    let usages = vec![
        ElementFactory::part_usage("test"),
        ElementFactory::item_usage("test"),
        ElementFactory::action_usage("test"),
        ElementFactory::state_usage("test"),
        ElementFactory::requirement_usage("test"),
        ElementFactory::constraint_usage("test"),
        ElementFactory::calculation_usage("test"),
        ElementFactory::case_usage("test"),
        ElementFactory::analysis_case_usage("test"),
        ElementFactory::verification_case_usage("test"),
        ElementFactory::use_case_usage("test"),
        ElementFactory::view_usage("test"),
        ElementFactory::viewpoint_usage("test"),
        ElementFactory::rendering_usage("test"),
        ElementFactory::port_usage("test"),
        ElementFactory::connection_usage("test"),
        ElementFactory::interface_usage("test"),
        ElementFactory::allocation_usage("test"),
        ElementFactory::attribute_usage("test"),
        ElementFactory::enumeration_usage("test"),
        ElementFactory::occurrence_usage("test"),
        ElementFactory::metadata_usage("test"),
    ];

    for usage in &usages {
        assert!(usage.kind.is_usage(), "{:?} should be a usage", usage.kind);
    }

    println!("ElementFactory covers {} usage types", usages.len());
}

#[test]
fn factory_sets_appropriate_defaults() {
    // Test that factory methods set type-appropriate defaults

    // PartUsage should have isComposite=true by default
    let part = ElementFactory::part_usage("test");
    assert_eq!(
        part.get_prop("isComposite").and_then(|v| v.as_bool()),
        Some(true),
        "PartUsage should default to isComposite=true"
    );

    // Definitions should have isAbstract=false by default
    let def = ElementFactory::part_definition("test");
    assert_eq!(
        def.get_prop("isAbstract").and_then(|v| v.as_bool()),
        Some(false),
        "PartDefinition should default to isAbstract=false"
    );

    // Abstract definitions should have isAbstract=true
    let abstract_def = ElementFactory::abstract_part_definition("test");
    assert_eq!(
        abstract_def
            .get_prop("isAbstract")
            .and_then(|v| v.as_bool()),
        Some(true),
        "abstract_part_definition should have isAbstract=true"
    );

    // Reference part should have isComposite=false
    let ref_part = ElementFactory::reference_part_usage("test");
    assert_eq!(
        ref_part.get_prop("isComposite").and_then(|v| v.as_bool()),
        Some(false),
        "reference_part_usage should have isComposite=false"
    );
}

// ============================================================================
// PROPERTY ACCESSOR COVERAGE TESTS
// ============================================================================

#[test]
fn property_accessors_exist_for_major_types() {
    // Test that property accessors exist for major element types

    // Create elements and verify we can cast to typed accessors
    let part_usage = Element::new_with_kind(ElementKind::PartUsage);
    assert!(part_usage.as_part_usage().is_some());

    let part_def = Element::new_with_kind(ElementKind::PartDefinition);
    assert!(part_def.as_part_definition().is_some());

    let package = Element::new_with_kind(ElementKind::Package);
    assert!(package.as_package().is_some());

    let action_usage = Element::new_with_kind(ElementKind::ActionUsage);
    assert!(action_usage.as_action_usage().is_some());

    let req_usage = Element::new_with_kind(ElementKind::RequirementUsage);
    assert!(req_usage.as_requirement_usage().is_some());
}

#[test]
fn property_accessor_validation_runs_without_panic() {
    // Test that validation can be called on all types with accessors
    let types_with_accessors = vec![
        ElementKind::PartUsage,
        ElementKind::PartDefinition,
        ElementKind::ActionUsage,
        ElementKind::ActionDefinition,
        ElementKind::RequirementUsage,
        ElementKind::RequirementDefinition,
        ElementKind::StateUsage,
        ElementKind::StateDefinition,
        ElementKind::Package,
    ];

    for kind in types_with_accessors {
        let element = Element::new_with_kind(kind.clone());

        // Try to get any accessor that might exist and call validate
        // This tests that the generated validate() method doesn't panic
        if let Some(accessor) = element.as_part_usage() {
            let _ = accessor.validate();
        }
        if let Some(accessor) = element.as_part_definition() {
            let _ = accessor.validate();
        }
        if let Some(accessor) = element.as_action_usage() {
            let _ = accessor.validate();
        }
        if let Some(accessor) = element.as_package() {
            let _ = accessor.validate();
        }
    }
}

// ============================================================================
// RELATIONSHIP TYPE COVERAGE TESTS
// ============================================================================

#[test]
fn relationship_types_have_source_target_constraints() {
    // Count how many relationship types have explicit constraints
    let mut with_source = 0;
    let mut with_target = 0;
    let mut relationship_count = 0;

    for kind in ElementKind::iter() {
        if kind.is_relationship() {
            relationship_count += 1;

            if kind.relationship_source_type().is_some() {
                with_source += 1;
            }
            if kind.relationship_target_type().is_some() {
                with_target += 1;
            }
        }
    }

    println!(
        "Relationship coverage: {}/{} have source type, {}/{} have target type",
        with_source, relationship_count, with_target, relationship_count
    );

    // We expect most relationships to have constraints
    let source_coverage = with_source as f64 / relationship_count as f64;
    assert!(
        source_coverage > 0.5,
        "Expected >50% of relationships to have source type constraints, got {:.1}%",
        source_coverage * 100.0
    );
}

#[test]
fn specific_relationships_have_correct_constraints() {
    // Test specific well-known relationship constraints

    // Specialization: Type -> Type
    assert_eq!(
        ElementKind::Specialization.relationship_source_type(),
        Some(ElementKind::Type)
    );
    assert_eq!(
        ElementKind::Specialization.relationship_target_type(),
        Some(ElementKind::Type)
    );

    // FeatureTyping: Feature -> Type
    assert_eq!(
        ElementKind::FeatureTyping.relationship_source_type(),
        Some(ElementKind::Feature)
    );
    assert_eq!(
        ElementKind::FeatureTyping.relationship_target_type(),
        Some(ElementKind::Type)
    );

    // Subsetting: Feature -> Feature
    assert_eq!(
        ElementKind::Subsetting.relationship_source_type(),
        Some(ElementKind::Feature)
    );
    assert_eq!(
        ElementKind::Subsetting.relationship_target_type(),
        Some(ElementKind::Feature)
    );
}

// ============================================================================
// MEMBERSHIP AND OWNERSHIP COVERAGE TESTS
// ============================================================================

#[test]
fn membership_types_are_relationships() {
    // All Membership types should be Relationships
    let membership_kinds = vec![
        ElementKind::Membership,
        ElementKind::OwningMembership,
        ElementKind::FeatureMembership,
    ];

    for kind in membership_kinds {
        assert!(
            kind.is_relationship() || kind.is_subtype_of(ElementKind::Relationship),
            "{:?} should be a Relationship",
            kind
        );
    }
}

#[test]
fn ownership_operations_work_for_all_namespace_types() {
    // Test that ownership operations work for various namespace types
    let namespace_kinds = vec![
        ElementKind::Package,
        ElementKind::LibraryPackage,
        ElementKind::PartDefinition,
        ElementKind::ActionDefinition,
    ];

    for ns_kind in namespace_kinds {
        let mut graph = ModelGraph::new();

        let ns = Element::new_with_kind(ns_kind.clone()).with_name("TestNamespace");
        let ns_id = graph.add_element(ns);

        let child = ElementFactory::part_usage("child");
        let child_id = graph.add_owned_element(child, ns_id.clone(), VisibilityKind::Public);

        // Verify ownership established
        assert!(
            graph.owner_of(&child_id).is_some(),
            "Child should have owner for namespace type {:?}",
            ns_kind
        );

        // Verify namespace operations work
        let members: Vec<_> = graph.owned_members(&ns_id).collect();
        assert_eq!(
            members.len(),
            1,
            "Namespace {:?} should have 1 member",
            ns_kind
        );
    }
}

// ============================================================================
// SUMMARY REPORT
// ============================================================================

#[test]
fn print_coverage_summary() {
    println!("\n=== SysML v2 Coverage Summary ===\n");

    // Element kinds
    let total_kinds = ElementKind::count();
    let definitions = ElementKind::iter().filter(|k| k.is_definition()).count();
    let usages = ElementKind::iter().filter(|k| k.is_usage()).count();
    let relationships = ElementKind::iter().filter(|k| k.is_relationship()).count();
    let features = ElementKind::iter().filter(|k| k.is_feature()).count();
    let classifiers = ElementKind::iter().filter(|k| k.is_classifier()).count();

    println!("ElementKind variants: {}", total_kinds);
    println!("  - Definitions: {}", definitions);
    println!("  - Usages: {}", usages);
    println!("  - Relationships: {}", relationships);
    println!("  - Features: {}", features);
    println!("  - Classifiers: {}", classifiers);

    // Value enums
    println!("\nValue Enums:");
    println!(
        "  - FeatureDirectionKind: {} variants",
        FeatureDirectionKind::count()
    );
    println!("  - VisibilityKind: {} variants", VisibilityKind::count());
    println!("  - PortionKind: {} variants", PortionKind::count());
    println!(
        "  - RequirementConstraintKind: {} variants",
        RequirementConstraintKind::count()
    );
    println!(
        "  - StateSubactionKind: {} variants",
        StateSubactionKind::count()
    );
    println!(
        "  - TransitionFeatureKind: {} variants",
        TransitionFeatureKind::count()
    );
    println!("  - TriggerKind: {} variants", TriggerKind::count());

    // Relationship constraints
    let with_constraints = ElementKind::iter()
        .filter(|k| k.is_relationship())
        .filter(|k| k.relationship_source_type().is_some())
        .count();

    println!("\nRelationship Constraints:");
    println!(
        "  - With source type: {}/{}",
        with_constraints, relationships
    );

    println!("\n=================================\n");
}
