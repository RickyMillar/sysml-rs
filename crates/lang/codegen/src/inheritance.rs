//! Property inheritance resolution for SysML/KerML types.
//!
//! This module resolves property inheritance by walking the type hierarchy
//! (see [`TypeHierarchy`], the one home for hierarchy computation) and
//! collecting properties from all supertypes.

use crate::hierarchy_generator::TypeHierarchy;
use crate::shapes_parser::{PropertyInfo, ShapeInfo};
use std::collections::HashMap;

/// Resolved properties for a type, including inherited ones.
#[derive(Debug, Clone)]
pub struct ResolvedShape {
    /// The element type name.
    pub element_type: String,
    /// All properties for this type (own + inherited).
    pub properties: Vec<PropertyInfo>,
    /// Direct supertypes.
    pub supertypes: Vec<String>,
    /// Description from the shape.
    pub description: Option<String>,
}

/// Resolve property inheritance for all shapes.
///
/// For each shape, collects properties from all supertypes in the hierarchy.
/// Properties from subtypes override properties from supertypes with the same name.
pub fn resolve_inheritance(
    shapes: &[ShapeInfo],
    hierarchy: &TypeHierarchy,
) -> HashMap<String, ResolvedShape> {
    // Build a map from element type to shape
    let shape_by_type: HashMap<&str, &ShapeInfo> = shapes
        .iter()
        .map(|s| (s.element_type.as_str(), s))
        .collect();

    let mut resolved: HashMap<String, ResolvedShape> = HashMap::new();

    for shape in shapes {
        let mut all_props: HashMap<String, PropertyInfo> = HashMap::new();

        // Get all supertypes (transitive, in DFS pre-order: nearest first)
        let supertypes = hierarchy
            .get_supertypes(&shape.element_type)
            .unwrap_or(&[]);

        // Process supertypes in reverse order (most general first)
        // so that more specific types override general ones
        for supertype in supertypes.iter().rev() {
            if let Some(super_shape) = shape_by_type.get(supertype.as_str()) {
                for prop in &super_shape.properties {
                    all_props.insert(prop.name.clone(), prop.clone());
                }
            }
        }

        // Add own properties (override inherited)
        for prop in &shape.properties {
            all_props.insert(prop.name.clone(), prop.clone());
        }

        // Sort properties by name for consistent output
        let mut properties: Vec<PropertyInfo> = all_props.into_values().collect();
        properties.sort_by(|a, b| a.name.cmp(&b.name));

        // Get direct supertypes
        let direct_supertypes = hierarchy
            .get_direct_supertypes(&shape.element_type)
            .map(<[String]>::to_vec)
            .unwrap_or_default();

        resolved.insert(
            shape.element_type.clone(),
            ResolvedShape {
                element_type: shape.element_type.clone(),
                properties,
                supertypes: direct_supertypes,
                description: shape.description.clone(),
            },
        );
    }

    resolved
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::shapes_parser::{Cardinality, PropertyType};
    use crate::ttl_parser::TypeInfo;

    fn make_type(name: &str, supertypes: &[&str]) -> TypeInfo {
        TypeInfo {
            name: name.to_string(),
            supertypes: supertypes.iter().map(|s| s.to_string()).collect(),
            comment: None,
        }
    }

    fn make_prop(name: &str) -> PropertyInfo {
        PropertyInfo {
            name: name.to_string(),
            cardinality: Cardinality::ZeroOrOne,
            property_type: PropertyType::Any,
            read_only: false,
            description: None,
        }
    }

    fn make_shape(element_type: &str, props: &[&str]) -> ShapeInfo {
        ShapeInfo {
            element_type: element_type.to_string(),
            shape_name: format!("{}Shape", element_type),
            properties: props.iter().map(|p| make_prop(p)).collect(),
            property_refs: Vec::new(),
            description: None,
        }
    }

    #[test]
    fn test_resolve_inheritance() {
        let shapes = vec![
            make_shape("Element", &["elementId", "name"]),
            make_shape("Namespace", &["member", "ownedMember"]),
            make_shape("Type", &["feature", "ownedFeature"]),
        ];

        let types = vec![
            make_type("Element", &[]),
            make_type("Namespace", &["Element"]),
            make_type("Type", &["Namespace"]),
        ];
        let hierarchy = TypeHierarchy::new(&types, &[]);

        let resolved = resolve_inheritance(&shapes, &hierarchy);

        // Element should have only its own properties
        let element = &resolved["Element"];
        assert_eq!(element.properties.len(), 2);

        // Namespace should have Element's properties + its own
        let namespace = &resolved["Namespace"];
        assert_eq!(namespace.properties.len(), 4); // elementId, name, member, ownedMember

        // Type should have all properties from the hierarchy
        let type_shape = &resolved["Type"];
        assert_eq!(type_shape.properties.len(), 6);
        let prop_names: Vec<&str> = type_shape
            .properties
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert!(prop_names.contains(&"elementId"));
        assert!(prop_names.contains(&"name"));
        assert!(prop_names.contains(&"member"));
        assert!(prop_names.contains(&"feature"));
    }

    #[test]
    fn test_property_override() {
        // When a subtype defines a property with the same name,
        // it should override the supertype's property
        let shapes = vec![
            make_shape("Base", &["prop1"]),
            ShapeInfo {
                element_type: "Derived".to_owned(),
                shape_name: "DerivedShape".to_owned(),
                properties: vec![PropertyInfo {
                    name: "prop1".to_owned(),
                    cardinality: Cardinality::ExactlyOne, // Different from base
                    property_type: PropertyType::Bool,
                    read_only: false,
                    description: Some("Overridden".to_owned()),
                }],
                property_refs: Vec::new(),
                description: None,
            },
        ];

        let types = vec![make_type("Base", &[]), make_type("Derived", &["Base"])];
        let hierarchy = TypeHierarchy::new(&types, &[]);

        let resolved = resolve_inheritance(&shapes, &hierarchy);

        let derived = &resolved["Derived"];
        assert_eq!(derived.properties.len(), 1);
        assert_eq!(derived.properties[0].cardinality, Cardinality::ExactlyOne);
        assert_eq!(
            derived.properties[0].description,
            Some("Overridden".to_owned())
        );
    }
}
