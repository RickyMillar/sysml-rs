//! Generator for relationship source/target type constraint methods.
//!
//! This module generates methods on `ElementKind` that expose the expected
//! source and target types for relationship elements.

use crate::hierarchy_generator::TypeHierarchy;
use crate::ttl_parser::TypeInfo;
use crate::xmi_relationship_parser::XmiRelationshipConstraint;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

/// Relationship constraint with source and target types.
pub struct RelationshipConstraint {
    pub source_type: String,
    pub target_type: String,
}

impl RelationshipConstraint {
    pub fn new(source_type: impl Into<String>, target_type: impl Into<String>) -> Self {
        Self {
            source_type: source_type.into(),
            target_type: target_type.into(),
        }
    }
}

/// A single fallback constraint entry as deserialized from TOML.
#[derive(Debug, Deserialize)]
struct FallbackEntry {
    type_name: String,
    source_type: String,
    target_type: String,
}

/// Top-level TOML structure for the fallback constraints file.
#[derive(Debug, Deserialize)]
struct FallbackFile {
    constraint: Vec<FallbackEntry>,
}

/// Get fallback constraints for relationships not in JSON or as defaults.
/// These are loaded from `data/relationship_fallbacks.toml`.
#[allow(clippy::expect_used)] // Build-time: must panic if bundled TOML is invalid
fn get_fallback_constraints() -> HashMap<String, RelationshipConstraint> {
    let toml_str = include_str!("../data/relationship_fallbacks.toml");
    let file: FallbackFile =
        toml::from_str(toml_str).expect("failed to parse relationship_fallbacks.toml");

    file.constraint
        .into_iter()
        .map(|entry| {
            (
                entry.type_name,
                RelationshipConstraint::new(entry.source_type, entry.target_type),
            )
        })
        .collect()
}

/// Generate relationship constraint methods with XMI-derived constraints.
///
/// XMI constraints are the authoritative source. Fallback constraints are used
/// for any types not found in XMI (if any).
///
/// # Priority Order
/// 1. XMI constraints (authoritative, from metamodel)
/// 2. Fallback constraints (for types not in XMI)
/// 3. Default to Element (only if neither source has the type)
pub fn generate_relationship_methods_with_xmi(
    kerml_types: &[TypeInfo],
    sysml_types: &[TypeInfo],
    xmi_constraints: &HashMap<String, XmiRelationshipConstraint>,
) -> String {
    let mut output = String::new();

    output.push_str("\n// === Relationship Type Constraint Methods ===\n");
    output.push_str("// Generated from XMI metamodel files\n\n");

    // Build hierarchy to identify all relationship types
    let hierarchy = TypeHierarchy::new(kerml_types, sysml_types);

    // Get all type names
    let all_types: HashSet<&str> = kerml_types
        .iter()
        .chain(sysml_types.iter())
        .map(|t| t.name.as_str())
        .collect();

    // Find all relationship types (deduplicated + sorted — the one home)
    let relationship_types: Vec<&str> = hierarchy.relationship_type_names();

    // Get fallback constraints for types not in XMI
    let fallback_map = get_fallback_constraints();

    // Helper to get constraint for a relationship type
    // Priority: XMI > Fallback > Default (Element)
    let get_constraint = |rel_type: &str| -> (&str, &str) {
        // Priority 1: XMI constraint (authoritative)
        if let Some(c) = xmi_constraints.get(rel_type) {
            return (c.source_type.as_str(), c.target_type.as_str());
        }
        // Priority 2: Fallback constraint
        if let Some(c) = fallback_map.get(rel_type) {
            return (c.source_type.as_str(), c.target_type.as_str());
        }
        // Default: Element (only if neither source has it)
        ("Element", "Element")
    };

    output.push_str("impl ElementKind {\n");

    // relationship_source_type()
    output.push_str("    /// For relationship types, returns the expected source element type.\n");
    output.push_str("    ///\n");
    output.push_str("    /// The source type indicates what kind of element can be the source of this relationship.\n");
    output.push_str("    /// Returns `None` for non-relationship types.\n");
    output.push_str("    ///\n");
    output.push_str("    /// # Examples\n");
    output.push_str("    ///\n");
    output.push_str("    /// ```\n");
    output.push_str("    /// use sysml_core::ElementKind;\n");
    output.push_str("    ///\n");
    output.push_str("    /// assert_eq!(\n");
    output.push_str("    ///     ElementKind::FeatureTyping.relationship_source_type(),\n");
    output.push_str("    ///     Some(ElementKind::Feature)\n");
    output.push_str("    /// );\n");
    output.push_str("    /// assert_eq!(ElementKind::Element.relationship_source_type(), None);\n");
    output.push_str("    /// ```\n");
    output.push_str("    pub const fn relationship_source_type(&self) -> Option<ElementKind> {\n");
    output.push_str("        match self {\n");

    for rel_type in &relationship_types {
        let (source_type, _) = get_constraint(rel_type);

        // Only include if the source type exists in our type list
        if all_types.contains(source_type) {
            output.push_str(&format!(
                "            ElementKind::{} => Some(ElementKind::{}),\n",
                rel_type, source_type
            ));
        } else {
            output.push_str(&format!(
                "            ElementKind::{} => Some(ElementKind::Element),\n",
                rel_type
            ));
        }
    }

    output.push_str("            _ => None,\n");
    output.push_str("        }\n");
    output.push_str("    }\n\n");

    // relationship_target_type()
    output.push_str("    /// For relationship types, returns the expected target element type.\n");
    output.push_str("    ///\n");
    output.push_str("    /// The target type indicates what kind of element can be the target of this relationship.\n");
    output.push_str("    /// Returns `None` for non-relationship types.\n");
    output.push_str("    ///\n");
    output.push_str("    /// # Examples\n");
    output.push_str("    ///\n");
    output.push_str("    /// ```\n");
    output.push_str("    /// use sysml_core::ElementKind;\n");
    output.push_str("    ///\n");
    output.push_str("    /// assert_eq!(\n");
    output.push_str("    ///     ElementKind::FeatureTyping.relationship_target_type(),\n");
    output.push_str("    ///     Some(ElementKind::Type)\n");
    output.push_str("    /// );\n");
    output.push_str("    /// assert_eq!(ElementKind::Element.relationship_target_type(), None);\n");
    output.push_str("    /// ```\n");
    output.push_str("    pub const fn relationship_target_type(&self) -> Option<ElementKind> {\n");
    output.push_str("        match self {\n");

    for rel_type in &relationship_types {
        let (_, target_type) = get_constraint(rel_type);

        // Only include if the target type exists in our type list
        if all_types.contains(target_type) {
            output.push_str(&format!(
                "            ElementKind::{} => Some(ElementKind::{}),\n",
                rel_type, target_type
            ));
        } else {
            output.push_str(&format!(
                "            ElementKind::{} => Some(ElementKind::Element),\n",
                rel_type
            ));
        }
    }

    output.push_str("            _ => None,\n");
    output.push_str("        }\n");
    output.push_str("    }\n\n");

    output.push_str("}\n");

    output
}

/// Get the list of fallback constraint type names.
///
/// This is useful for coverage validation.
pub fn get_fallback_constraint_names() -> Vec<String> {
    get_fallback_constraints().into_keys().collect()
}

/// Information about a relationship's target property.
#[derive(Debug, Clone)]
pub struct RelationshipTargetProperty {
    /// The property name containing the target (e.g., "general", "type").
    pub property: String,
    /// Whether this is a multi-valued (list) property.
    pub is_multi: bool,
}

/// Source property names that represent the "source" side of a relationship.
/// These are the owning features, not the targets we want to validate.
const SOURCE_PROPERTY_PATTERNS: &[&str] = &[
    "specific",                  // Specialization - the specializing type
    "subsettingFeature",         // Subsetting - the subsetting feature
    "redefiningFeature",         // Redefinition - the redefining feature
    "featureInverted",           // FeatureInverting - the inverted feature
    "typedFeature",              // FeatureTyping - the typed feature
    "typeDisjoined",             // Disjoining - the disjoined type
    "subclassifier",             // Subclassification - the subclassifier
    "featureOfType",             // TypeFeaturing - the featured type
    "annotatingElement",         // Annotation - the annotating element (source)
    "importOwningNamespace",     // Import - the importing namespace
    "owningRelatedElement",      // Generic - owner element
    "membershipOwningNamespace", // Membership - owning namespace
];

/// Map grammar rule names to element type names where they differ.
///
/// The Xtext grammar often uses fragment names or abbreviated rule names
/// that differ from the actual element type names.
fn normalize_rule_to_element_type(rule_name: &str) -> String {
    // First strip "Owned" prefix if present
    let base = rule_name.strip_prefix("Owned").unwrap_or(rule_name);

    // Map grammar fragment/rule names to ElementKind names
    match base {
        // FeatureType fragment returns FeatureTyping
        "FeatureType" => "FeatureTyping".to_owned(),
        // MetadataTyping returns FeatureTyping (but for metadata)
        "MetadataTyping" => "FeatureTyping".to_owned(),
        // ConjugatedPortTyping rule -> ConjugatedPortTyping element
        "ConjugatedPortTyping" => "ConjugatedPortTyping".to_owned(),
        // CrossSubsetting is a feature chain subsetting
        "CrossSubsetting" => "CrossSubsetting".to_owned(),
        // Keep the base name for most rules
        _ => base.to_owned(),
    }
}

/// Build a map from relationship type names to their target property info.
///
/// Uses cross-reference data from Xtext grammar to identify which property
/// contains the target element reference.
pub fn build_relationship_target_properties(
    cross_refs: &[crate::xtext_crossref_parser::CrossReference],
) -> HashMap<String, RelationshipTargetProperty> {
    let mut result: HashMap<String, RelationshipTargetProperty> = HashMap::new();

    for cr in cross_refs {
        // Skip source-side properties (these are the owning/source features)
        if is_source_property(&cr.property) {
            continue;
        }

        // Normalize rule name to element type name
        let element_type = normalize_rule_to_element_type(&cr.containing_rule);

        // Only add if we don't already have an entry (first occurrence wins)
        result
            .entry(element_type)
            .or_insert_with(|| RelationshipTargetProperty {
                property: cr.property.clone(),
                is_multi: cr.is_multi,
            });
    }

    result
}

/// Check if a property name represents the source side of a relationship.
fn is_source_property(property: &str) -> bool {
    SOURCE_PROPERTY_PATTERNS.contains(&property)
}

/// Coverage report for relationship target property mappings.
#[derive(Debug)]
pub struct RelationshipPropertyCoverageReport {
    /// Total number of relationship types.
    pub total_relationships: usize,
    /// Number of relationships with target property mappings.
    pub with_mapping: usize,
    /// Relationship types without mappings.
    pub without_mapping: Vec<String>,
    /// Coverage percentage.
    pub coverage_percent: f64,
}

/// Validate coverage of relationship target property mappings.
pub fn validate_relationship_property_coverage(
    relationship_types: &[&str],
    property_map: &HashMap<String, RelationshipTargetProperty>,
) -> RelationshipPropertyCoverageReport {
    let mut without_mapping = Vec::new();
    let mut with_mapping = 0;

    for rel_type in relationship_types {
        if property_map.contains_key(*rel_type) {
            with_mapping += 1;
        } else {
            without_mapping.push(rel_type.to_string());
        }
    }

    let total = relationship_types.len();
    let coverage_percent = if total > 0 {
        (with_mapping as f64 / total as f64) * 100.0
    } else {
        100.0
    };

    RelationshipPropertyCoverageReport {
        total_relationships: total,
        with_mapping,
        without_mapping,
        coverage_percent,
    }
}

/// Generate relationship target property methods for ElementKind.
///
/// Generates:
/// - `relationship_target_property()` - Returns the property name containing the target
/// - `relationship_target_is_list()` - Returns whether the target property is a list
pub fn generate_relationship_property_methods(
    kerml_types: &[TypeInfo],
    sysml_types: &[TypeInfo],
    property_map: &HashMap<String, RelationshipTargetProperty>,
) -> String {
    let mut output = String::new();

    output.push_str("\n// === Relationship Target Property Methods ===\n");
    output.push_str("// Generated from Xtext cross-reference registry\n\n");

    // Build hierarchy to identify all relationship types
    let hierarchy = TypeHierarchy::new(kerml_types, sysml_types);

    // Find all relationship types (deduplicated + sorted — the one home)
    let relationship_types: Vec<&str> = hierarchy.relationship_type_names();

    output.push_str("impl ElementKind {\n");

    // relationship_target_property()
    output.push_str("    /// For relationship types, returns the property name containing the target element.\n");
    output.push_str("    ///\n");
    output.push_str(
        "    /// This property name can be used to look up the target ElementId in the\n",
    );
    output.push_str("    /// relationship element's props map after name resolution.\n");
    output.push_str("    ///\n");
    output.push_str("    /// # Examples\n");
    output.push_str("    ///\n");
    output.push_str("    /// ```\n");
    output.push_str("    /// use sysml_core::ElementKind;\n");
    output.push_str("    ///\n");
    output.push_str("    /// assert_eq!(ElementKind::Specialization.relationship_target_property(), Some(\"general\"));\n");
    output.push_str("    /// assert_eq!(ElementKind::FeatureTyping.relationship_target_property(), Some(\"type\"));\n");
    output.push_str(
        "    /// assert_eq!(ElementKind::Element.relationship_target_property(), None);\n",
    );
    output.push_str("    /// ```\n");
    output.push_str(
        "    pub const fn relationship_target_property(&self) -> Option<&'static str> {\n",
    );
    output.push_str("        match self {\n");

    for rel_type in &relationship_types {
        if let Some(prop_info) = property_map.get(*rel_type) {
            output.push_str(&format!(
                "            ElementKind::{} => Some(\"{}\"),\n",
                rel_type, prop_info.property
            ));
        }
    }

    output.push_str("            _ => None,\n");
    output.push_str("        }\n");
    output.push_str("    }\n\n");

    // relationship_target_is_list()
    output.push_str(
        "    /// For relationship types, returns whether the target property is a list.\n",
    );
    output.push_str("    ///\n");
    output.push_str(
        "    /// Most relationships have a single target, but some (like Dependency.supplier)\n",
    );
    output.push_str("    /// can have multiple targets.\n");
    output.push_str("    ///\n");
    output.push_str("    /// # Examples\n");
    output.push_str("    ///\n");
    output.push_str("    /// ```\n");
    output.push_str("    /// use sysml_core::ElementKind;\n");
    output.push_str("    ///\n");
    output.push_str(
        "    /// assert_eq!(ElementKind::Dependency.relationship_target_is_list(), true);\n",
    );
    output.push_str(
        "    /// assert_eq!(ElementKind::Specialization.relationship_target_is_list(), false);\n",
    );
    output.push_str("    /// ```\n");
    output.push_str("    pub const fn relationship_target_is_list(&self) -> bool {\n");
    output.push_str("        match self {\n");

    // Only output entries for list properties to keep the match arm small
    for rel_type in &relationship_types {
        if let Some(prop_info) = property_map.get(*rel_type) {
            if prop_info.is_multi {
                output.push_str(&format!("            ElementKind::{} => true,\n", rel_type));
            }
        }
    }

    output.push_str("            _ => false,\n");
    output.push_str("        }\n");
    output.push_str("    }\n");

    output.push_str("}\n");

    output
}


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn make_type(name: &str, supertypes: &[&str]) -> TypeInfo {
        TypeInfo {
            name: name.to_string(),
            supertypes: supertypes.iter().map(|s| s.to_string()).collect(),
            comment: None,
        }
    }

    fn base_kerml_types() -> Vec<TypeInfo> {
        vec![
            make_type("Element", &[]),
            make_type("Type", &["Element"]),
            make_type("Feature", &["Type"]),
            make_type("Classifier", &["Type"]),
            make_type("Namespace", &["Element"]),
            make_type("Relationship", &["Element"]),
            make_type("Specialization", &["Relationship"]),
            make_type("FeatureTyping", &["Specialization"]),
        ]
    }

    #[test]
    fn test_generates_relationship_methods() {
        let kerml = base_kerml_types();
        let sysml = vec![];

        let code = generate_relationship_methods_with_xmi(&kerml, &sysml, &HashMap::new());

        // Check method signatures
        assert!(code.contains("pub const fn relationship_source_type(&self)"));
        assert!(code.contains("pub const fn relationship_target_type(&self)"));

        // Check known constraints are applied
        assert!(code.contains("ElementKind::FeatureTyping => Some(ElementKind::Feature)"));
        assert!(code.contains("ElementKind::Specialization => Some(ElementKind::Type)"));

        // Non-relationship types should return None
        assert!(code.contains("_ => None,"));
    }

    #[test]
    fn test_identifies_relationship_subtypes() {
        let kerml = vec![
            make_type("Element", &[]),
            make_type("Relationship", &["Element"]),
            make_type("Annotation", &["Relationship"]),
        ];
        let sysml = vec![];

        let code = generate_relationship_methods_with_xmi(&kerml, &sysml, &HashMap::new());

        // Annotation should be included as it's a Relationship subtype
        assert!(code.contains("ElementKind::Annotation"));
    }

    #[test]
    fn fallback_used_when_no_json_constraints() {
        let kerml = base_kerml_types();
        let sysml = vec![];

        // No XMI constraints provided → fallback should be used
        let code = generate_relationship_methods_with_xmi(&kerml, &sysml, &HashMap::new());

        // FeatureTyping fallback: source=Feature, target=Type
        assert!(
            code.contains("ElementKind::FeatureTyping => Some(ElementKind::Feature)"),
            "fallback should set FeatureTyping source to Feature"
        );
    }

    #[test]
    fn unknown_type_falls_back_to_element() {
        let kerml = vec![
            make_type("Element", &[]),
            make_type("Relationship", &["Element"]),
            make_type("CustomRel", &["Relationship"]),
        ];
        let sysml = vec![];

        // CustomRel is not in fallback map, should default to Element
        let code = generate_relationship_methods_with_xmi(&kerml, &sysml, &HashMap::new());

        assert!(
            code.contains("ElementKind::CustomRel => Some(ElementKind::Element)"),
            "unknown relationship type should default to Element"
        );
    }

    #[test]
    fn membership_containment_constraint() {
        let kerml = vec![
            make_type("Element", &[]),
            make_type("Namespace", &["Element"]),
            make_type("Relationship", &["Element"]),
            make_type("Membership", &["Relationship"]),
        ];
        let sysml = vec![];

        let code = generate_relationship_methods_with_xmi(&kerml, &sysml, &HashMap::new());

        // Membership fallback: source=Namespace, target=Element
        assert!(
            code.contains("ElementKind::Membership => Some(ElementKind::Namespace)"),
            "Membership source should be Namespace"
        );
    }

    #[test]
    fn empty_relationship_types_generates_wildcard_only() {
        // No relationship types at all
        let kerml = vec![make_type("Element", &[]), make_type("Type", &["Element"])];
        let sysml = vec![];

        let code = generate_relationship_methods_with_xmi(&kerml, &sysml, &HashMap::new());

        // Should still generate the methods with just the wildcard arm
        assert!(code.contains("pub const fn relationship_source_type(&self)"));
        assert!(code.contains("_ => None,"));
        // But no specific ElementKind:: entries
        assert!(
            !code.contains("ElementKind::Relationship"),
            "no relationship types means no match arms"
        );
    }

    #[test]
    fn bidirectional_source_and_target_generated() {
        let kerml = vec![
            make_type("Element", &[]),
            make_type("Type", &["Element"]),
            make_type("Feature", &["Type"]),
            make_type("Relationship", &["Element"]),
            make_type("Specialization", &["Relationship"]),
        ];
        let sysml = vec![];

        let code = generate_relationship_methods_with_xmi(&kerml, &sysml, &HashMap::new());

        // Specialization: source=Type, target=Type (from fallback)
        // Both source and target methods should reference Specialization
        let source_count = code.matches("ElementKind::Specialization").count();
        assert!(
            source_count >= 2,
            "Specialization should appear in both source and target methods, found {} occurrences",
            source_count
        );
    }

    #[test]
    fn toml_fallback_loads_exactly_57_entries() {
        let constraints = get_fallback_constraints();
        assert_eq!(
            constraints.len(),
            57,
            "Expected 57 fallback constraints from TOML, got {}",
            constraints.len()
        );
    }

    #[test]
    fn toml_fallback_spot_check_entries() {
        let constraints = get_fallback_constraints();

        // Check a few known entries
        let rel = constraints
            .get("Relationship")
            .expect("Relationship must exist");
        assert_eq!(rel.source_type, "Element");
        assert_eq!(rel.target_type, "Element");

        let ft = constraints
            .get("FeatureTyping")
            .expect("FeatureTyping must exist");
        assert_eq!(ft.source_type, "Feature");
        assert_eq!(ft.target_type, "Type");

        let om = constraints
            .get("ObjectiveMembership")
            .expect("ObjectiveMembership must exist");
        assert_eq!(om.source_type, "CaseDefinition");
        assert_eq!(om.target_type, "RequirementUsage");

        let pc = constraints
            .get("PortConjugation")
            .expect("PortConjugation must exist");
        assert_eq!(pc.source_type, "ConjugatedPortDefinition");
        assert_eq!(pc.target_type, "PortDefinition");

        let inter = constraints
            .get("Interaction")
            .expect("Interaction must exist");
        assert_eq!(inter.source_type, "Type");
        assert_eq!(inter.target_type, "Type");
    }

    #[test]
    fn get_fallback_constraint_names_matches_toml_count() {
        let names = get_fallback_constraint_names();
        assert_eq!(names.len(), 57);
        assert!(names.contains(&"Relationship".to_owned()));
        assert!(names.contains(&"Interaction".to_owned()));
    }
}
