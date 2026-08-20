// Build scripts use panic/expect as the standard way to report build failures.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
//! Build script for sysml-core.
//!
//! This script generates:
//! 1. The `ElementKind` enum from KerML and SysML vocabulary TTL files
//! 2. Type hierarchy methods (supertypes, predicates, definition/usage mappings)
//! 3. Relationship source/target constraint methods (from JSON schemas)
//! 4. Value enumeration types (FeatureDirectionKind, VisibilityKind, etc.)
//! 5. Typed property accessors from OSLC shapes files
//!
//! ## Spec Coverage Validation
//!
//! The build script validates that the generated code has complete coverage
//! of the SysML v2 / KerML specifications by cross-checking multiple sources:
//! - TTL vocabulary files vs XMI metamodel files (type coverage)
//! - TTL enum values vs JSON schema enum values (enum coverage)
//!
//! The build will **fail** if:
//! - Types exist in XMI but not in TTL (or vice versa)
//! - Enum values differ between TTL and JSON schema

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

fn main() {
    // Determine paths relative to the manifest directory
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let repo_root = Path::new(&manifest_dir).join("../../..");
    let refs_dir = find_references_dir(&repo_root);

    let kerml_vocab_path = refs_dir.join("Kerml-Vocab.ttl");
    let sysml_vocab_path = refs_dir.join("SysML-vocab.ttl");
    let kerml_shapes_path = refs_dir.join("KerML-shapes.ttl");
    let sysml_shapes_path = refs_dir.join("SysML-shapes.ttl");
    let json_schema_dir = if refs_dir.join("metamodel").is_dir() {
        refs_dir.join("metamodel")
    } else {
        refs_dir.join("SysML-v2-API-Services/conf/json/schema/metamodel")
    };
    let kerml_xmi_path = refs_dir.join("KerML/20250201/KerML.xmi");
    let sysml_xmi_path = refs_dir.join("SysML/20250201/SysML.xmi");

    // Re-run if any source files change
    println!("cargo:rerun-if-changed={}", kerml_vocab_path.display());
    println!("cargo:rerun-if-changed={}", sysml_vocab_path.display());
    println!("cargo:rerun-if-changed={}", kerml_shapes_path.display());
    println!("cargo:rerun-if-changed={}", sysml_shapes_path.display());
    println!("cargo:rerun-if-changed={}", json_schema_dir.display());
    // XMI files for relationship constraints (authoritative source)
    println!("cargo:rerun-if-changed={}", kerml_xmi_path.display());
    println!("cargo:rerun-if-changed={}", sysml_xmi_path.display());
    println!("cargo:rerun-if-changed=build.rs");

    // Read the vocabulary files
    let kerml_vocab_content = fs::read_to_string(&kerml_vocab_path)
        .unwrap_or_else(|e| panic!("Failed to read {:?}: {}", kerml_vocab_path, e));
    let sysml_vocab_content = fs::read_to_string(&sysml_vocab_path)
        .unwrap_or_else(|e| panic!("Failed to read {:?}: {}", sysml_vocab_path, e));

    // Parse the vocabularies (types)
    let kerml_types = sysml_codegen::parse_ttl_vocab(&kerml_vocab_content)
        .unwrap_or_else(|e| panic!("Failed to parse KerML vocab: {}", e));
    let sysml_types = sysml_codegen::parse_ttl_vocab(&sysml_vocab_content)
        .unwrap_or_else(|e| panic!("Failed to parse SysML vocab: {}", e));

    // Parse the vocabularies (enums)
    let kerml_enums = sysml_codegen::parse_ttl_enums(&kerml_vocab_content)
        .unwrap_or_else(|e| panic!("Failed to parse KerML enums: {}", e));
    let sysml_enums = sysml_codegen::parse_ttl_enums(&sysml_vocab_content)
        .unwrap_or_else(|e| panic!("Failed to parse SysML enums: {}", e));

    // Merge enums from both sources (deduplicating)
    let all_enums = sysml_codegen::merge_enum_info(kerml_enums.clone(), sysml_enums.clone());

    println!(
        "cargo:warning=Parsed {} KerML enums and {} SysML enums, merged to {} unique enums",
        kerml_enums.len(),
        sysml_enums.len(),
        all_enums.len()
    );

    // =====================================================================
    // SPEC COVERAGE VALIDATION
    // =====================================================================
    // Validate that our codegen pipeline has complete coverage of the spec
    // by cross-checking multiple authoritative sources.
    //
    // The build will FAIL if:
    // - Types exist in XMI but not in TTL (or vice versa)
    // - Enum values differ between TTL and JSON schema
    // =====================================================================

    // Parse XMI classes for type coverage validation
    let xmi_classes = sysml_codegen::parse_all_xmi_classes(&kerml_xmi_path, &sysml_xmi_path)
        .unwrap_or_else(|e| panic!("Failed to parse XMI classes: {}", e));

    // Collect TTL type names for validation
    // Exclude enum types (names ending with "Kind") since they are defined as
    // uml:Enumeration in XMI, not uml:Class, so they won't be in xmi_classes
    let ttl_type_names: Vec<String> = kerml_types
        .iter()
        .chain(sysml_types.iter())
        .filter(|t| !t.name.ends_with("Kind"))
        .map(|t| t.name.clone())
        .collect();

    // Validate type coverage (TTL vs XMI)
    let type_report = sysml_codegen::validate_type_coverage(&ttl_type_names, &xmi_classes);

    println!(
        "cargo:warning=Type coverage: {} TTL, {} XMI, {} matched",
        type_report.ttl_total, type_report.xmi_total, type_report.matched_count
    );

    if type_report.has_errors() {
        if !type_report.ttl_only.is_empty() {
            println!(
                "cargo:warning=Types in TTL but not XMI ({}): {:?}",
                type_report.ttl_only.len(),
                type_report.ttl_only
            );
        }
        if !type_report.xmi_only.is_empty() {
            println!(
                "cargo:warning=Types in XMI but not TTL ({}): {:?}",
                type_report.xmi_only.len(),
                type_report.xmi_only
            );
        }
        // Fail the build on type coverage errors
        panic!(
            "TYPE COVERAGE VALIDATION FAILED: {} errors\n\
             TTL-only types: {:?}\n\
             XMI-only types: {:?}",
            type_report.error_count(),
            type_report.ttl_only,
            type_report.xmi_only
        );
    }

    // Parse JSON enums for enum coverage validation
    let json_enums = sysml_codegen::parse_all_enums_from_json(&json_schema_dir);

    // Convert TTL enums to the format needed for validation
    let ttl_enums_for_validation: Vec<(String, Vec<String>)> = all_enums
        .iter()
        .map(|e| {
            (
                e.name.clone(),
                e.values.iter().map(|v| v.name.clone()).collect(),
            )
        })
        .collect();

    // Convert JSON enums to the format needed for validation
    let json_enums_for_validation: Vec<(String, Vec<String>)> = json_enums
        .iter()
        .map(|e| (e.name.clone(), e.values.clone()))
        .collect();

    // Validate enum coverage (TTL vs JSON)
    let enum_report = sysml_codegen::validate_enum_coverage(
        &ttl_enums_for_validation,
        &json_enums_for_validation,
    );

    println!(
        "cargo:warning=Enum coverage: {} TTL enums, {} JSON enums",
        ttl_enums_for_validation.len(),
        json_enums_for_validation.len()
    );

    if enum_report.has_errors() {
        let mut error_details = String::new();
        if !enum_report.ttl_only_enums.is_empty() {
            error_details.push_str(&format!(
                "Enums in TTL but not JSON: {:?}\n",
                enum_report.ttl_only_enums
            ));
        }
        if !enum_report.json_only_enums.is_empty() {
            error_details.push_str(&format!(
                "Enums in JSON but not TTL: {:?}\n",
                enum_report.json_only_enums
            ));
        }
        for enum_val in &enum_report.enums {
            if !enum_val.is_valid() {
                error_details.push_str(&format!(
                    "{}: TTL-only values {:?}, JSON-only values {:?}\n",
                    enum_val.name, enum_val.ttl_only, enum_val.json_only
                ));
            }
        }
        // Fail the build on enum coverage errors
        panic!(
            "ENUM COVERAGE VALIDATION FAILED: {} errors\n{}",
            enum_report.error_count(),
            error_details
        );
    }

    println!("cargo:warning=Spec coverage validation PASSED");

    // =====================================================================
    // CROSS-REFERENCE COVERAGE VALIDATION
    // =====================================================================
    // Validate that all cross-references in the Xtext grammar are handled
    // by the name resolution implementation.
    //
    // This ensures we don't miss any reference types when implementing
    // name resolution.
    // =====================================================================

    // Paths to Xtext grammar files
    // Note: Xtext files live in references (not spec/)
    let xtext_refs_dir = find_references_dir(&repo_root);
    let pilot_impl_dir = xtext_refs_dir.join("SysML-v2-Pilot-Implementation");
    let kerml_xtext_path =
        pilot_impl_dir.join("org.omg.kerml.xtext/src/org/omg/kerml/xtext/KerML.xtext");
    let sysml_xtext_path =
        pilot_impl_dir.join("org.omg.sysml.xtext/src/org/omg/sysml/xtext/SysML.xtext");

    // Re-run if Xtext files change
    println!("cargo:rerun-if-changed={}", kerml_xtext_path.display());
    println!("cargo:rerun-if-changed={}", sysml_xtext_path.display());

    // Parse cross-references from Xtext grammars
    let mut all_cross_refs = Vec::new();

    if kerml_xtext_path.exists() {
        let kerml_xtext_content = fs::read_to_string(&kerml_xtext_path)
            .unwrap_or_else(|e| panic!("Failed to read {:?}: {}", kerml_xtext_path, e));
        let kerml_refs =
            sysml_codegen::parse_xtext_cross_references(&kerml_xtext_content, "KerML.xtext");
        println!(
            "cargo:warning=Parsed {} cross-references from KerML.xtext",
            kerml_refs.len()
        );
        all_cross_refs.extend(kerml_refs);
    } else {
        println!(
            "cargo:warning=KerML.xtext not found at {:?}, skipping cross-reference validation",
            kerml_xtext_path
        );
    }

    if sysml_xtext_path.exists() {
        let sysml_xtext_content = fs::read_to_string(&sysml_xtext_path)
            .unwrap_or_else(|e| panic!("Failed to read {:?}: {}", sysml_xtext_path, e));
        let sysml_refs =
            sysml_codegen::parse_xtext_cross_references(&sysml_xtext_content, "SysML.xtext");
        println!(
            "cargo:warning=Parsed {} cross-references from SysML.xtext",
            sysml_refs.len()
        );
        all_cross_refs.extend(sysml_refs);
    } else {
        println!(
            "cargo:warning=SysML.xtext not found at {:?}, skipping cross-reference validation",
            sysml_xtext_path
        );
    }

    // Deduplicate cross-references (same property from both files)
    let unique_properties: std::collections::HashSet<String> = all_cross_refs
        .iter()
        .map(|cr| cr.property.clone())
        .collect();
    println!(
        "cargo:warning=Total unique cross-reference properties: {}",
        unique_properties.len()
    );

    // Currently implemented cross-reference properties in resolution module
    // These are the properties that have resolution logic in src/resolution/mod.rs
    const IMPLEMENTED_CROSSREFS: &[&str] = &[
        // From unresolved_props constants in resolution/mod.rs
        "general",           // Specialization.general
        "type",              // FeatureTyping.type
        "subsettedFeature",  // Subsetting.subsettedFeature
        "redefinedFeature",  // Redefinition.redefinedFeature
        "referencedFeature", // ReferenceSubsetting.referencedFeature
        // Note: "sources" and "targets" are for Dependency but stored differently
        // Note: "value" is for FeatureValue but not a cross-reference

        // Phase B: Additional cross-references
        "superclassifier",          // Subclassification.superclassifier
        "conjugatedType",           // Conjugation.conjugatedType
        "originalType",             // Conjugation.originalType
        "featuringType",            // TypeFeaturing.featuringType
        "disjoiningType",           // Disjoining.disjoiningType
        "unioningType",             // Unioning.unioningType
        "intersectingType",         // Intersecting.intersectingType
        "differencingType",         // Differencing.differencingType
        "invertingFeature",         // FeatureInverting.invertingFeature
        "crossedFeature",           // FeatureChaining.crossedFeature (chainingFeature)
        "annotatedElement",         // Annotation.annotatedElement
        "memberElement",            // Membership.memberElement
        "client",                   // Dependency.client
        "supplier",                 // Dependency.supplier
        "conjugatedPortDefinition", // ConjugatedPortDefinition.conjugatedPortDefinition
    ];

    // Properties that are intentionally skipped (with reasons in crossref_validation.rs)
    // Add properties here that don't need resolution or are handled differently
    const SKIPPED_CROSSREFS: &[&str] = &[
        // Parser-internal properties (not stored in model)
        "specific",          // Specialization - the owning feature, not resolved
        "subsettingFeature", // Subsetting - the owning feature
        "redefiningFeature", // Redefinition - the owning feature
        "featureInverted",   // FeatureInverting - the owning feature
        "typedFeature",      // FeatureTyping - the owning feature
        "typeDisjoined",     // Disjoining - the owning type
        "subclassifier",     // Subclassification - the owning classifier
        "featureOfType",     // TypeFeaturing - the owning feature
    ];

    // Validate coverage
    let impl_and_skipped: Vec<&str> = IMPLEMENTED_CROSSREFS
        .iter()
        .chain(SKIPPED_CROSSREFS.iter())
        .copied()
        .collect();
    let report = sysml_codegen::validate_crossref_coverage(&all_cross_refs, &impl_and_skipped);

    println!(
        "cargo:warning=Cross-reference coverage: {}/{} ({:.1}%)",
        report.handled.len(),
        report.grammar_total,
        report.coverage_percent()
    );

    if !report.unhandled.is_empty() {
        println!(
            "cargo:warning=Unhandled cross-references ({}): {:?}",
            report.unhandled.len(),
            report.unhandled
        );
        // Now that we have 100% coverage, enforce it going forward
        panic!(
            "CROSS-REFERENCE COVERAGE FAILED!\n\
             {} unhandled properties: {:?}\n\
             Add resolution logic or mark as intentionally skipped.",
            report.unhandled.len(),
            report.unhandled
        );
    }

    if !report.extra.is_empty() {
        println!(
            "cargo:warning=Extra cross-references not in grammar: {:?}",
            report.extra
        );
    }

    // =====================================================================
    // RESOLUTION SPEC COMPLETENESS VALIDATION
    // =====================================================================
    // Validate that every unresolved_* property in the resolution module
    // has a corresponding entry in the cross-reference registry.
    //
    // This ensures spec completeness: all resolvable properties have
    // explicit scoping strategies defined.
    // =====================================================================

    // Read the resolution module source
    let resolution_mod_path = Path::new(&manifest_dir).join("src/resolution/mod.rs");
    println!("cargo:rerun-if-changed={}", resolution_mod_path.display());

    if resolution_mod_path.exists() {
        let resolution_mod_content = fs::read_to_string(&resolution_mod_path)
            .unwrap_or_else(|e| panic!("Failed to read {:?}: {}", resolution_mod_path, e));

        // Extract unresolved_* properties from resolution module
        let resolution_props =
            sysml_codegen::extract_resolution_unresolved_props(&resolution_mod_content);

        println!(
            "cargo:warning=Resolution module defines {} unresolved_* properties",
            resolution_props.len()
        );

        // Get registry property names
        let registry_props: Vec<String> = all_cross_refs
            .iter()
            .map(|cr| cr.property.clone())
            .collect();

        // Validate with mappings (sources->source, targets->target) and exclusions (value)
        let spec_result = sysml_codegen::validate_resolution_spec_with_mappings(
            &resolution_props,
            &registry_props,
        );

        println!(
            "cargo:warning=Resolution Spec Completeness: {}/{} properties validated ({:.1}%)",
            spec_result.validated.len(),
            spec_result.resolution_total,
            spec_result.coverage_percent()
        );

        println!(
            "cargo:warning=  Phase A (parser-created): {}",
            spec_result.phase_a_props.len()
        );
        println!(
            "cargo:warning=  Phase B (resolution-only): {}",
            spec_result.phase_b_props.len()
        );

        if !spec_result.missing_from_registry.is_empty() {
            println!(
                "cargo:warning=Resolution properties missing from registry: {:?}",
                spec_result.missing_from_registry
            );
        }

        // Fail build if strict validation is enabled and there are missing entries
        if env::var("SYSML_STRICT_VALIDATION").is_ok() && !spec_result.is_valid() {
            panic!(
                "RESOLUTION SPEC COMPLETENESS FAILED!\n\
                 {} properties missing from registry: {:?}\n\
                 Add corresponding entries to the cross-reference registry.",
                spec_result.missing_from_registry.len(),
                spec_result.missing_from_registry
            );
        }
    } else {
        println!(
            "cargo:warning=Resolution module not found at {:?}, skipping spec validation",
            resolution_mod_path
        );
    }

    // =====================================================================
    // CODE GENERATION
    // =====================================================================

    // Generate the ElementKind enum
    let enum_code = sysml_codegen::generate_enum("ElementKind", &kerml_types, &sysml_types);

    // Generate type hierarchy methods
    let hierarchy_code = sysml_codegen::generate_hierarchy_methods(&kerml_types, &sysml_types);

    // The one queryable type hierarchy, used by the relationship filter,
    // property-inheritance resolution, and the semantic dispatcher below.
    let type_hierarchy = sysml_codegen::TypeHierarchy::new(&kerml_types, &sysml_types);

    // The one definition of "relationship type": Relationship + transitive
    // subtypes (so FeatureTyping → Specialization → Relationship counts).
    let relationship_type_names: Vec<&str> = type_hierarchy.relationship_type_names();

    // Parse XMI files for relationship constraints (authoritative source)
    // Note: kerml_xmi_path and sysml_xmi_path are already declared in spec validation section
    let xmi_constraints =
        sysml_codegen::parse_relationship_constraints(&kerml_xmi_path, &sysml_xmi_path)
            .unwrap_or_else(|e| panic!("Failed to parse XMI relationship constraints: {}", e));

    // Validate coverage
    let fallback_names = sysml_codegen::get_fallback_constraint_names();
    let coverage = sysml_codegen::validate_relationship_coverage(
        &relationship_type_names,
        &xmi_constraints,
        &fallback_names,
    );

    println!(
        "cargo:warning=Relationship constraints: {}/{} from XMI, {} from fallback",
        coverage.from_xmi, coverage.total, coverage.from_fallback
    );

    if !coverage.missing.is_empty() {
        println!(
            "cargo:warning=Relationship types without constraints (using default Element): {:?}",
            coverage.missing
        );
    }

    // Generate relationship constraint methods with XMI-parsed constraints
    let relationship_code = sysml_codegen::generate_relationship_methods_with_xmi(
        &kerml_types,
        &sysml_types,
        &xmi_constraints,
    );

    // =====================================================================
    // RELATIONSHIP TARGET PROPERTY GENERATION (Phase 4)
    // =====================================================================
    // Generate methods to look up which property contains the target element
    // for each relationship type. This enables validate_relationship_types().
    // =====================================================================

    // Build map from relationship types to their target property info
    let property_map = sysml_codegen::build_relationship_target_properties(&all_cross_refs);

    // Validate coverage
    let property_coverage = sysml_codegen::validate_relationship_property_coverage(
        &relationship_type_names,
        &property_map,
    );

    println!(
        "cargo:warning=Relationship property mappings: {}/{} ({:.1}%)",
        property_coverage.with_mapping,
        property_coverage.total_relationships,
        property_coverage.coverage_percent
    );

    if !property_coverage.without_mapping.is_empty() {
        println!(
            "cargo:warning=Relationships without target property mapping: {:?}",
            property_coverage.without_mapping
        );
    }

    // Generate property lookup methods
    let relationship_property_code = sysml_codegen::generate_relationship_property_methods(
        &kerml_types,
        &sysml_types,
        &property_map,
    );

    // Combine all ElementKind-related code
    let element_kind_code = format!(
        "{}\n{}\n{}\n{}",
        enum_code, hierarchy_code, relationship_code, relationship_property_code
    );

    // Collect valid element kinds for filtering
    let mut valid_kinds: HashSet<String> = HashSet::new();
    for t in &kerml_types {
        valid_kinds.insert(t.name.clone());
    }
    for t in &sysml_types {
        valid_kinds.insert(t.name.clone());
    }

    // Write ElementKind enum and methods
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
    let enum_path = Path::new(&out_dir).join("element_kind.generated.rs");
    fs::write(&enum_path, &element_kind_code)
        .unwrap_or_else(|e| panic!("Failed to write {:?}: {}", enum_path, e));

    // Generate value enums
    let enums_code = sysml_codegen::generate_value_enums(&all_enums);
    let enums_path = Path::new(&out_dir).join("enums.generated.rs");
    fs::write(&enums_path, &enums_code)
        .unwrap_or_else(|e| panic!("Failed to write {:?}: {}", enums_path, e));

    println!(
        "cargo:warning=Generated ElementKind enum with {} KerML and {} SysML types",
        kerml_types.len(),
        sysml_types.len()
    );
    println!(
        "cargo:warning=Generated {} value enumeration types",
        all_enums.len()
    );

    // Now parse shapes files for property accessors
    let kerml_shapes_content = fs::read_to_string(&kerml_shapes_path)
        .unwrap_or_else(|e| panic!("Failed to read {:?}: {}", kerml_shapes_path, e));
    let sysml_shapes_content = fs::read_to_string(&sysml_shapes_path)
        .unwrap_or_else(|e| panic!("Failed to read {:?}: {}", sysml_shapes_path, e));

    // Parse KerML shapes
    let (mut kerml_shapes, kerml_shared_props) =
        sysml_codegen::shapes_parser::parse_oslc_shapes(&kerml_shapes_content)
            .unwrap_or_else(|e| panic!("Failed to parse KerML shapes: {}", e));

    // Resolve shared properties for KerML
    sysml_codegen::shapes_parser::resolve_shared_properties(&mut kerml_shapes, &kerml_shared_props);

    // Parse SysML shapes
    let (mut sysml_shapes, sysml_shared_props) =
        sysml_codegen::shapes_parser::parse_oslc_shapes(&sysml_shapes_content)
            .unwrap_or_else(|e| panic!("Failed to parse SysML shapes: {}", e));

    // Resolve shared properties for SysML
    sysml_codegen::shapes_parser::resolve_shared_properties(&mut sysml_shapes, &sysml_shared_props);

    // Merge all shared properties (KerML ones might be referenced by SysML)
    let mut all_shared_props = kerml_shared_props;
    all_shared_props.extend(sysml_shared_props);

    // Also resolve SysML shapes with KerML shared properties
    sysml_codegen::shapes_parser::resolve_shared_properties(&mut sysml_shapes, &all_shared_props);

    // Combine shapes
    let mut all_shapes = kerml_shapes;
    all_shapes.extend(sysml_shapes);

    println!(
        "cargo:warning=Parsed {} shapes ({} KerML, {} SysML) with {} shared properties",
        all_shapes.len(),
        all_shapes
            .iter()
            .filter(|s| s.element_type.starts_with(|c: char| c.is_uppercase()))
            .count(),
        all_shapes.len(),
        all_shared_props.len()
    );

    // Resolve property inheritance (walks the shared type hierarchy)
    let resolved_shapes =
        sysml_codegen::inheritance::resolve_inheritance(&all_shapes, &type_hierarchy);

    // Filter to only shapes that have corresponding ElementKind variants
    let valid_kinds_vec: Vec<String> = valid_kinds.into_iter().collect();
    let filtered_resolved: std::collections::HashMap<_, _> = resolved_shapes
        .into_iter()
        .filter(|(k, _)| valid_kinds_vec.contains(k))
        .collect();

    println!(
        "cargo:warning=Generating accessors for {} shapes with valid ElementKind",
        filtered_resolved.len()
    );

    // Generate property accessors
    let accessor_code =
        sysml_codegen::accessor_generator::generate_property_accessors(&filtered_resolved);

    // Generate validation methods
    let validation_code =
        sysml_codegen::validation_generator::generate_validation_methods(&filtered_resolved);

    // =====================================================================
    // PROPERTY VALIDATION COVERAGE
    // =====================================================================
    // Analyze property constraints from shapes and report which validation
    // constraint types are covered by the generated validation code.
    // This provides visibility into validation gaps at build time.
    // =====================================================================

    let coverage_result = sysml_codegen::validate_property_validation_coverage(&filtered_resolved);

    println!(
        "cargo:warning=Property validation coverage: {}/{} constraint types ({:.1}%)",
        coverage_result.implemented_constraints.len(),
        coverage_result.implemented_constraints.len() + coverage_result.missing_constraints.len(),
        coverage_result.coverage_percent()
    );

    // Report per-constraint-type statistics
    for constraint_type in sysml_codegen::ConstraintType::all() {
        if let Some(stats) = coverage_result.constraint_stats.get(constraint_type) {
            if stats.applicable_count > 0 {
                let status = if stats.is_implemented {
                    "COVERED"
                } else {
                    "MISSING"
                };
                println!(
                    "cargo:warning=  {:?}: {} properties, {}",
                    constraint_type, stats.applicable_count, status
                );
            }
        }
    }

    if !coverage_result.missing_constraints.is_empty() {
        println!(
            "cargo:warning=Unimplemented validation constraints: {:?}",
            coverage_result.missing_constraints
        );
    }

    // Generate property validation dispatcher
    let dispatcher_code =
        sysml_codegen::validation_generator::generate_property_validation_dispatcher(
            &filtered_resolved,
        );

    // Combine into properties.generated.rs
    let mut properties_code = String::new();
    properties_code.push_str(&accessor_code);
    properties_code.push_str("\n// Validation methods\n\n");
    properties_code.push_str(&validation_code);
    properties_code.push('\n');
    properties_code.push_str(&dispatcher_code);

    // Write properties file
    let props_path = Path::new(&out_dir).join("properties.generated.rs");
    fs::write(&props_path, &properties_code)
        .unwrap_or_else(|e| panic!("Failed to write {:?}: {}", props_path, e));

    println!(
        "cargo:warning=Generated property accessors in {:?}",
        props_path
    );

    // Generate cross-reference registry
    let crossref_code = sysml_codegen::generate_crossref_registry(&all_cross_refs);
    let crossref_path = Path::new(&out_dir).join("crossrefs.generated.rs");
    fs::write(&crossref_path, &crossref_code)
        .unwrap_or_else(|e| panic!("Failed to write {:?}: {}", crossref_path, e));

    println!(
        "cargo:warning=Generated cross-reference registry with {} properties",
        all_cross_refs.len()
    );

    // =====================================================================
    // SEMANTIC VALIDATION DISPATCHER
    // =====================================================================
    // Parse semantic_rules.toml and generate a validation dispatcher that
    // calls hand-implemented check functions in semantic_checks/ module.
    // =====================================================================

    let semantic_rules_path = Path::new(&manifest_dir).join("../codegen/src/semantic_rules.toml");
    println!("cargo:rerun-if-changed={}", semantic_rules_path.display());

    if semantic_rules_path.exists() {
        let semantic_rules = sysml_codegen::parse_semantic_rules_file(&semantic_rules_path)
            .unwrap_or_else(|e| panic!("Failed to parse semantic rules: {}", e));

        let summary = sysml_codegen::summarize_rules(&semantic_rules);

        println!(
            "cargo:warning=Semantic validation rules: {} total ({} errors, {} warnings) across {} categories",
            summary.total_rules,
            summary.error_count,
            summary.warning_count,
            summary.categories.len()
        );

        println!(
            "cargo:warning=Semantic check functions: {} unique",
            summary.check_functions.len()
        );

        // Coverage tracking: scan semantic_checks/ directory for implemented functions
        let checks_dir = Path::new(&manifest_dir).join("src/semantic_checks");
        if checks_dir.exists() {
            let mut implemented_fns: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            if let Ok(entries) = fs::read_dir(&checks_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().is_some_and(|ext| ext == "rs")
                        && path.file_name().is_some_and(|n| n != "mod.rs")
                    {
                        if let Ok(content) = fs::read_to_string(&path) {
                            for line in content.lines() {
                                let trimmed = line.trim();
                                if trimmed.starts_with("pub fn ")
                                    && !trimmed.contains("fn setup_")
                                    && !trimmed.contains("fn count_")
                                    && !trimmed.contains("fn find_")
                                    && !trimmed.contains("fn check_")
                                    && !trimmed.contains("fn is_")
                                    && !trimmed.contains("fn has_")
                                    && !trimmed.contains("fn typing_error")
                                    && !trimmed.contains("fn spec_error")
                                    && !trimmed.contains("fn end_count_error")
                                    && !trimmed.contains("fn cardinality_error")
                                {
                                    if let Some(fn_name) = trimmed
                                        .strip_prefix("pub fn ")
                                        .and_then(|s| s.split('(').next())
                                    {
                                        implemented_fns.insert(fn_name.to_owned());
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let required: std::collections::HashSet<String> =
                summary.check_functions.iter().cloned().collect();
            let covered = required.intersection(&implemented_fns).count();
            let missing: Vec<_> = required
                .difference(&implemented_fns)
                .cloned()
                .collect::<Vec<_>>();
            let coverage_pct = if required.is_empty() {
                100.0
            } else {
                (covered as f64 / required.len() as f64) * 100.0
            };

            println!(
                "cargo:warning=Semantic validation coverage: {}/{} check functions ({:.0}%)",
                covered,
                required.len(),
                coverage_pct,
            );
            if !missing.is_empty() {
                let mut sorted_missing = missing;
                sorted_missing.sort();
                println!(
                    "cargo:warning=Missing check functions: {}",
                    sorted_missing.join(", ")
                );
            }
        }

        // Generate the semantic validation dispatcher (hierarchy-aware): a rule
        // registered on a supertype ElementKind also applies to its subtypes, so
        // e.g. at-most-one-objective (registered on CaseDefinition/CaseUsage) fires
        // on UseCaseDefinition/VerificationCaseDefinition/AnalysisCaseDefinition
        // subtypes (SysML §8.3.22.2). Check functions self-guard with
        // `is_subtype_of`, so the supertype match arm does not double-fire.
        let semantic_validation_code = sysml_codegen::generate_semantic_validation_with_hierarchy(
            &semantic_rules,
            Some(&type_hierarchy),
        );

        let semantic_path = Path::new(&out_dir).join("semantic_validation.generated.rs");
        fs::write(&semantic_path, &semantic_validation_code)
            .unwrap_or_else(|e| panic!("Failed to write {:?}: {}", semantic_path, e));

        println!(
            "cargo:warning=Generated semantic validation dispatcher in {:?}",
            semantic_path
        );
    } else {
        // Write a stub if no rules file exists
        let semantic_path = Path::new(&out_dir).join("semantic_validation.generated.rs");
        fs::write(
            &semantic_path,
            "// No semantic_rules.toml found - semantic validation disabled\n\
             pub fn validate_semantic(_graph: &ModelGraph) -> Vec<SemanticError> {\n\
                 Vec::new()\n\
             }\n",
        )
        .unwrap_or_else(|e| panic!("Failed to write {:?}: {}", semantic_path, e));
    }
}

fn find_references_dir(repo_root: &Path) -> PathBuf {
    if let Ok(refs_dir) = env::var("SYSML_REFS_DIR") {
        let path = PathBuf::from(refs_dir);
        if path.exists() {
            return path;
        }
    }

    let in_repo = repo_root.join("references").join("sysmlv2");
    if in_repo.exists() {
        return in_repo;
    }

    panic!(
        "Could not find references/sysmlv2/ directory under repo root: {}",
        repo_root.display()
    );
}
