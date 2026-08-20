//! Integration test for parsing real TTL vocabulary, shapes files, and XMI files.

use std::fs;
use std::path::Path;
use sysml_codegen::inheritance::resolve_inheritance;
use sysml_codegen::TypeHierarchy;
use sysml_codegen::shapes_parser::{parse_oslc_shapes, resolve_shared_properties};
use sysml_codegen::{
    generate_enum, get_fallback_constraint_names, parse_relationship_constraints, parse_ttl_vocab,
    validate_relationship_coverage,
};

/// Test parsing the real KerML vocabulary file.
#[test]
fn parse_kerml_vocab() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../references/sysmlv2/Kerml-Vocab.ttl");
    let content = fs::read_to_string(path).expect("Failed to read Kerml-Vocab.ttl");

    let types = parse_ttl_vocab(&content).expect("Failed to parse KerML vocab");

    // According to the spec, KerML should have 84 types
    // Allow some flexibility in case the spec is updated
    assert!(
        types.len() >= 80,
        "Expected at least 80 KerML types, got {}",
        types.len()
    );
    assert!(
        types.len() <= 100,
        "Expected at most 100 KerML types, got {}",
        types.len()
    );

    // Check for some known types
    let names: Vec<&str> = types.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"Element"), "Should contain Element");
    assert!(
        names.contains(&"Relationship"),
        "Should contain Relationship"
    );
    assert!(names.contains(&"Feature"), "Should contain Feature");
    assert!(names.contains(&"Type"), "Should contain Type");
    assert!(names.contains(&"Classifier"), "Should contain Classifier");
    assert!(names.contains(&"Namespace"), "Should contain Namespace");

    println!("Parsed {} KerML types", types.len());
}

/// Test parsing the real SysML vocabulary file.
#[test]
fn parse_sysml_vocab() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../references/sysmlv2/SysML-vocab.ttl");
    let content = fs::read_to_string(path).expect("Failed to read SysML-vocab.ttl");

    let types = parse_ttl_vocab(&content).expect("Failed to parse SysML vocab");

    // According to the spec, SysML should have 182 types
    // Allow some flexibility in case the spec is updated
    assert!(
        types.len() >= 170,
        "Expected at least 170 SysML types, got {}",
        types.len()
    );
    assert!(
        types.len() <= 200,
        "Expected at most 200 SysML types, got {}",
        types.len()
    );

    // Check for some known types
    let names: Vec<&str> = types.iter().map(|t| t.name.as_str()).collect();
    assert!(
        names.contains(&"PartDefinition"),
        "Should contain PartDefinition"
    );
    assert!(names.contains(&"PartUsage"), "Should contain PartUsage");
    assert!(
        names.contains(&"ActionDefinition"),
        "Should contain ActionDefinition"
    );
    assert!(names.contains(&"ActionUsage"), "Should contain ActionUsage");
    assert!(
        names.contains(&"RequirementDefinition"),
        "Should contain RequirementDefinition"
    );
    assert!(
        names.contains(&"RequirementUsage"),
        "Should contain RequirementUsage"
    );

    println!("Parsed {} SysML types", types.len());
}

/// Test generating the full enum from both vocabularies.
#[test]
fn generate_full_enum() {
    let kerml_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../references/sysmlv2/Kerml-Vocab.ttl");
    let sysml_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../references/sysmlv2/SysML-vocab.ttl");

    let kerml_content = fs::read_to_string(kerml_path).expect("Failed to read Kerml-Vocab.ttl");
    let sysml_content = fs::read_to_string(sysml_path).expect("Failed to read SysML-vocab.ttl");

    let kerml_types = parse_ttl_vocab(&kerml_content).expect("Failed to parse KerML vocab");
    let sysml_types = parse_ttl_vocab(&sysml_content).expect("Failed to parse SysML vocab");

    let code = generate_enum("ElementKind", &kerml_types, &sysml_types);

    // Verify the generated code contains expected structure
    assert!(
        code.contains("pub enum ElementKind"),
        "Should have enum definition"
    );
    assert!(
        code.contains("// === KerML Types ==="),
        "Should have KerML section"
    );
    assert!(
        code.contains("// === SysML Types ==="),
        "Should have SysML section"
    );
    assert!(code.contains("pub fn iter()"), "Should have iter method");
    assert!(
        code.contains("pub fn as_str(&self)"),
        "Should have as_str method"
    );
    assert!(
        code.contains("pub fn from_str(s: &str)"),
        "Should have from_str method"
    );
    assert!(
        code.contains("pub const fn count()"),
        "Should have count method"
    );

    // Verify some specific types
    assert!(code.contains("Element,"), "Should have Element variant");
    assert!(code.contains("PartUsage,"), "Should have PartUsage variant");
    assert!(
        code.contains("ActionDefinition,"),
        "Should have ActionDefinition variant"
    );

    // Check the generated count matches the total unique types
    let total = kerml_types.len()
        + sysml_types
            .iter()
            .filter(|t| !kerml_types.iter().any(|k| k.name == t.name))
            .count();

    println!("Generated enum with {} total types", total);
    assert!(
        code.contains(&format!("{}", total)),
        "Should have correct count in count() method"
    );
}

/// Test that generated code is valid Rust by checking for balanced braces.
#[test]
fn generated_code_is_balanced() {
    let kerml_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../references/sysmlv2/Kerml-Vocab.ttl");
    let sysml_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../references/sysmlv2/SysML-vocab.ttl");

    let kerml_content = fs::read_to_string(kerml_path).expect("Failed to read Kerml-Vocab.ttl");
    let sysml_content = fs::read_to_string(sysml_path).expect("Failed to read SysML-vocab.ttl");

    let kerml_types = parse_ttl_vocab(&kerml_content).expect("Failed to parse KerML vocab");
    let sysml_types = parse_ttl_vocab(&sysml_content).expect("Failed to parse SysML vocab");

    let code = generate_enum("ElementKind", &kerml_types, &sysml_types);

    // Check for balanced braces (critical for Rust syntax)
    let open_braces = code.matches('{').count();
    let close_braces = code.matches('}').count();
    assert_eq!(open_braces, close_braces, "Braces should be balanced");

    // Check for balanced brackets (critical for Rust syntax)
    let open_brackets = code.matches('[').count();
    let close_brackets = code.matches(']').count();
    assert_eq!(open_brackets, close_brackets, "Brackets should be balanced");

    // Note: We don't check parentheses because doc comments may contain
    // unbalanced parens from the specification text (e.g., "(in the universe)")
}

/// Test parsing the real KerML shapes file.
#[test]
fn parse_kerml_shapes() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../references/sysmlv2/KerML-shapes.ttl");
    let content = fs::read_to_string(path).expect("Failed to read KerML-shapes.ttl");

    let (shapes, shared_props) = parse_oslc_shapes(&content).expect("Failed to parse KerML shapes");

    // KerML shapes should have ~83 shapes
    println!(
        "Parsed {} KerML shapes with {} shared properties",
        shapes.len(),
        shared_props.len()
    );
    assert!(
        shapes.len() >= 70,
        "Expected at least 70 KerML shapes, got {}",
        shapes.len()
    );
    assert!(
        shapes.len() <= 100,
        "Expected at most 100 KerML shapes, got {}",
        shapes.len()
    );

    // Check for some known shapes
    let element_shape = shapes.iter().find(|s| s.element_type == "Element");
    assert!(element_shape.is_some(), "Should have Element shape");

    let element = element_shape.unwrap();
    assert!(
        element.properties.len() > 0 || element.property_refs.len() > 0,
        "Element shape should have properties"
    );

    // Check shared properties include common ones
    assert!(
        shared_props.contains_key("elementId"),
        "Should have elementId shared property"
    );
    assert!(
        shared_props.contains_key("name"),
        "Should have name shared property"
    );
}

/// Test parsing the real SysML shapes file.
#[test]
fn parse_sysml_shapes() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../references/sysmlv2/SysML-shapes.ttl");
    let content = fs::read_to_string(path).expect("Failed to read SysML-shapes.ttl");

    let (shapes, shared_props) = parse_oslc_shapes(&content).expect("Failed to parse SysML shapes");

    // SysML shapes should have ~176 shapes
    println!(
        "Parsed {} SysML shapes with {} shared properties",
        shapes.len(),
        shared_props.len()
    );
    assert!(
        shapes.len() >= 150,
        "Expected at least 150 SysML shapes, got {}",
        shapes.len()
    );
    assert!(
        shapes.len() <= 200,
        "Expected at most 200 SysML shapes, got {}",
        shapes.len()
    );

    // Check for some known shapes
    let part_shape = shapes.iter().find(|s| s.element_type == "PartUsage");
    assert!(part_shape.is_some(), "Should have PartUsage shape");

    let action_def_shape = shapes.iter().find(|s| s.element_type == "ActionDefinition");
    assert!(
        action_def_shape.is_some(),
        "Should have ActionDefinition shape"
    );
}

/// Test resolving property inheritance.
#[test]
fn test_inheritance_resolution() {
    // Parse vocab for type hierarchy
    let kerml_vocab_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../references/sysmlv2/Kerml-Vocab.ttl");
    let sysml_vocab_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../references/sysmlv2/SysML-vocab.ttl");

    let kerml_vocab = fs::read_to_string(kerml_vocab_path).expect("Failed to read Kerml-Vocab.ttl");
    let sysml_vocab = fs::read_to_string(sysml_vocab_path).expect("Failed to read SysML-vocab.ttl");

    let kerml_types = parse_ttl_vocab(&kerml_vocab).expect("Failed to parse KerML vocab");
    let sysml_types = parse_ttl_vocab(&sysml_vocab).expect("Failed to parse SysML vocab");

    // Parse shapes
    let kerml_shapes_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../references/sysmlv2/KerML-shapes.ttl");
    let kerml_shapes_content =
        fs::read_to_string(kerml_shapes_path).expect("Failed to read KerML-shapes.ttl");

    let (mut kerml_shapes, kerml_shared) =
        parse_oslc_shapes(&kerml_shapes_content).expect("Failed to parse KerML shapes");

    resolve_shared_properties(&mut kerml_shapes, &kerml_shared);

    // Build hierarchy
    let hierarchy = TypeHierarchy::new(&kerml_types, &sysml_types);

    // Resolve inheritance
    let resolved = resolve_inheritance(&kerml_shapes, &hierarchy);

    // Feature should inherit from Type, Namespace, Element
    if let Some(feature_shape) = resolved.get("Feature") {
        // Feature should have more properties than just its own due to inheritance
        println!(
            "Feature has {} properties after inheritance resolution",
            feature_shape.properties.len()
        );
        assert!(
            feature_shape.properties.len() > 5,
            "Feature should have inherited properties, got {}",
            feature_shape.properties.len()
        );
    }

    // Type should inherit from Namespace, Element
    if let Some(type_shape) = resolved.get("Type") {
        println!(
            "Type has {} properties after inheritance resolution",
            type_shape.properties.len()
        );
        assert!(
            type_shape.properties.len() > 5,
            "Type should have inherited properties"
        );
    }

    println!("Resolved inheritance for {} shapes", resolved.len());
}

/// Test parsing the real XMI files for relationship constraints.
#[test]
fn parse_xmi_relationship_constraints() {
    let kerml_xmi = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../references/sysmlv2/KerML/20250201/KerML.xmi"
    );
    let sysml_xmi = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../references/sysmlv2/SysML/20250201/SysML.xmi"
    );

    let constraints = parse_relationship_constraints(Path::new(kerml_xmi), Path::new(sysml_xmi))
        .expect("Failed to parse XMI relationship constraints");

    // We should have constraints for relationship types that explicitly redefine source/target
    // Not all relationship types redefine - many inherit from their parent type
    println!(
        "Parsed {} relationship constraints from XMI",
        constraints.len()
    );

    // Print all constraint names for debugging
    let mut names: Vec<_> = constraints.keys().collect();
    names.sort();
    println!("Constraints found: {:?}", names);

    assert!(
        constraints.len() >= 15,
        "Expected at least 15 XMI constraints (types with explicit redefinition), got {}",
        constraints.len()
    );

    // Check for known relationship type constraints
    // Specialization: source=Type, target=Type (directly redefines Relationship source/target)
    if let Some(spec) = constraints.get("Specialization") {
        assert_eq!(
            spec.source_type, "Type",
            "Specialization source should be Type"
        );
        assert_eq!(
            spec.target_type, "Type",
            "Specialization target should be Type"
        );
        assert!(
            spec.source_from_xmi,
            "Specialization source should be from XMI"
        );
        assert!(
            spec.target_from_xmi,
            "Specialization target should be from XMI"
        );
    } else {
        panic!("Should have Specialization constraint");
    }

    // Connector: source=Feature, target=Feature (directly redefines)
    if let Some(conn) = constraints.get("Connector") {
        assert_eq!(
            conn.source_type, "Feature",
            "Connector source should be Feature"
        );
        assert_eq!(
            conn.target_type, "Feature",
            "Connector target should be Feature"
        );
    } else {
        panic!("Should have Connector constraint");
    }

    // Note: FeatureTyping doesn't directly redefine Relationship.source/target,
    // it redefines Specialization.specific/general, so it won't be in the XMI constraints.
    // That's expected - the generator handles inheritance by falling back to the fallback map.

    // Subclassification: should redefine from Specialization
    if let Some(sub) = constraints.get("Subclassification") {
        assert_eq!(
            sub.source_type, "Classifier",
            "Subclassification source should be Classifier"
        );
        assert_eq!(
            sub.target_type, "Classifier",
            "Subclassification target should be Classifier"
        );
    } else {
        // Subclassification may also not directly redefine Relationship.source/target
        println!("Note: Subclassification not found in XMI constraints (may inherit from Specialization)");
    }

    // FeatureMembership should have constraints (directly redefines from Membership)
    if let Some(fm) = constraints.get("FeatureMembership") {
        println!(
            "FeatureMembership: source={}, target={}",
            fm.source_type, fm.target_type
        );
    }
}

/// Test coverage validation for relationship constraints.
#[test]
fn test_xmi_constraint_coverage() {
    let kerml_xmi = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../references/sysmlv2/KerML/20250201/KerML.xmi"
    );
    let sysml_xmi = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../references/sysmlv2/SysML/20250201/SysML.xmi"
    );
    let kerml_vocab_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../references/sysmlv2/Kerml-Vocab.ttl");
    let sysml_vocab_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../references/sysmlv2/SysML-vocab.ttl");

    // Parse vocabularies
    let kerml_vocab = fs::read_to_string(kerml_vocab_path).expect("Failed to read Kerml-Vocab.ttl");
    let sysml_vocab = fs::read_to_string(sysml_vocab_path).expect("Failed to read SysML-vocab.ttl");
    let kerml_types = parse_ttl_vocab(&kerml_vocab).expect("Failed to parse KerML vocab");
    let sysml_types = parse_ttl_vocab(&sysml_vocab).expect("Failed to parse SysML vocab");

    // Get relationship types — the one home's definition, identical to what
    // the generator emits.
    let hierarchy = TypeHierarchy::new(&kerml_types, &sysml_types);
    let relationship_type_names: Vec<&str> = hierarchy.relationship_type_names();

    // Parse XMI constraints
    let xmi_constraints =
        parse_relationship_constraints(Path::new(kerml_xmi), Path::new(sysml_xmi))
            .expect("Failed to parse XMI");

    // Validate coverage
    let fallback_names = get_fallback_constraint_names();
    let coverage =
        validate_relationship_coverage(&relationship_type_names, &xmi_constraints, &fallback_names);

    println!(
        "Coverage: {}/{} from XMI, {} from fallback",
        coverage.from_xmi, coverage.total, coverage.from_fallback
    );
    println!("Missing constraints: {:?}", coverage.missing);

    // Every relationship type without an XMI or fallback constraint falls to
    // the Element/Element default in the generated methods. Pin that set
    // exactly so a NEW relationship type can't silently join it — extend the
    // fallback TOML (or XMI parsing) instead of this list.
    let expected_element_default: std::collections::BTreeSet<&str> = [
        "AllocationDefinition",
        "ConnectionDefinition",
        "ConnectorAsUsage",
        "FeatureValue",
        "Flow",
        "FlowDefinition",
        "FlowUsage",
        "InterfaceDefinition",
        "MembershipExpose",
        "NamespaceExpose",
        "SuccessionFlow",
        "SuccessionFlowUsage",
    ]
    .into_iter()
    .collect();
    let actual_missing: std::collections::BTreeSet<&str> =
        coverage.missing.iter().map(String::as_str).collect();
    assert_eq!(
        actual_missing, expected_element_default,
        "relationship types falling to the Element/Element default changed; \
         add a fallback constraint rather than growing this list"
    );
}
