//! Property accessor code generator.
//!
//! This module generates typed property accessor structs and methods
//! from resolved shape information.

use crate::inheritance::ResolvedShape;
use crate::shapes_parser::{Cardinality, PropertyInfo, PropertyType};
use std::collections::HashMap;

/// Convert a SysML property name to a Rust method name (snake_case).
fn to_snake_case(name: &str) -> String {
    let mut result = String::new();
    let mut prev_lower = false;

    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if prev_lower || (i > 0 && i < name.len() - 1) {
                // Check if next char is lowercase (e.g., "XMLLiteral" -> "xml_literal")
                let next_lower = name
                    .chars()
                    .nth(i + 1)
                    .map(|c| c.is_lowercase())
                    .unwrap_or(false);
                if prev_lower || next_lower {
                    result.push('_');
                }
            }
            result.push(c.to_ascii_lowercase());
            prev_lower = false;
        } else {
            result.push(c);
            prev_lower = true;
        }
    }

    // Handle Rust keywords
    match result.as_str() {
        "type" => "type_".to_owned(),
        "ref" => "ref_".to_owned(),
        "mod" => "mod_".to_owned(),
        "self" => "self_".to_owned(),
        "super" => "super_".to_owned(),
        "crate" => "crate_".to_owned(),
        "extern" => "extern_".to_owned(),
        "trait" => "trait_".to_owned(),
        "impl" => "impl_".to_owned(),
        "fn" => "fn_".to_owned(),
        "let" => "let_".to_owned(),
        "mut" => "mut_".to_owned(),
        "const" => "const_".to_owned(),
        "static" => "static_".to_owned(),
        "where" => "where_".to_owned(),
        "use" => "use_".to_owned(),
        "as" => "as_".to_owned(),
        "if" => "if_".to_owned(),
        "else" => "else_".to_owned(),
        "match" => "match_".to_owned(),
        "loop" => "loop_".to_owned(),
        "while" => "while_".to_owned(),
        "for" => "for_".to_owned(),
        "in" => "in_".to_owned(),
        "return" => "return_".to_owned(),
        "break" => "break_".to_owned(),
        "continue" => "continue_".to_owned(),
        _ => result,
    }
}

/// Convert a type name to a Rust struct name for the accessor (e.g., "PartUsage" -> "PartUsageProps").
fn accessor_struct_name(element_type: &str) -> String {
    format!("{}Props", element_type)
}

/// Convert a type name to a cast method name (e.g., "PartUsage" -> "as_part_usage").
fn cast_method_name(element_type: &str) -> String {
    format!("as_{}", to_snake_case(element_type))
}

/// Generate the return type for a property based on cardinality.
fn property_return_type(prop: &PropertyInfo) -> String {
    let base_type = match &prop.property_type {
        PropertyType::Bool => "bool".to_owned(),
        PropertyType::String => "String".to_owned(),
        PropertyType::DateTime => "String".to_owned(), // Store as string
        PropertyType::ElementRef(type_name) => format!("ElementId /* {} */", type_name),
        PropertyType::Any => "Value".to_owned(),
    };

    match prop.cardinality {
        Cardinality::ExactlyOne => {
            if matches!(prop.property_type, PropertyType::Bool) {
                "bool".to_owned()
            } else {
                format!("Option<{}>", base_type)
            }
        }
        Cardinality::ZeroOrOne => format!("Option<{}>", base_type),
        Cardinality::ZeroOrMany | Cardinality::OneOrMany => {
            if matches!(prop.property_type, PropertyType::ElementRef(_)) {
                "impl Iterator<Item = ElementId> + '_".to_owned()
            } else {
                format!("impl Iterator<Item = {}> + '_", base_type)
            }
        }
    }
}

/// Generate the method body for accessing a property.
fn property_accessor_body(prop: &PropertyInfo, struct_ref: &str) -> String {
    let prop_name = &prop.name;

    match prop.cardinality {
        Cardinality::ExactlyOne => match &prop.property_type {
            PropertyType::Bool => {
                format!(
                    "{}.props.get(\"{}\").and_then(|v| v.as_bool()).unwrap_or(false)",
                    struct_ref, prop_name
                )
            }
            PropertyType::String | PropertyType::DateTime => {
                format!(
                    "{}.props.get(\"{}\").and_then(|v| v.as_str()).map(|s| s.to_string())",
                    struct_ref, prop_name
                )
            }
            PropertyType::ElementRef(_) => {
                format!(
                    "{}.props.get(\"{}\").and_then(|v| v.as_ref()).cloned()",
                    struct_ref, prop_name
                )
            }
            PropertyType::Any => {
                format!("{}.props.get(\"{}\").cloned()", struct_ref, prop_name)
            }
        },
        Cardinality::ZeroOrOne => match &prop.property_type {
            PropertyType::Bool => {
                format!(
                    "{}.props.get(\"{}\").and_then(|v| v.as_bool())",
                    struct_ref, prop_name
                )
            }
            PropertyType::String | PropertyType::DateTime => {
                format!(
                    "{}.props.get(\"{}\").and_then(|v| v.as_str()).map(|s| s.to_string())",
                    struct_ref, prop_name
                )
            }
            PropertyType::ElementRef(_) => {
                format!(
                    "{}.props.get(\"{}\").and_then(|v| v.as_ref()).cloned()",
                    struct_ref, prop_name
                )
            }
            PropertyType::Any => {
                format!("{}.props.get(\"{}\").cloned()", struct_ref, prop_name)
            }
        },
        Cardinality::ZeroOrMany | Cardinality::OneOrMany => match &prop.property_type {
            PropertyType::Bool => {
                format!(
                        "{}.props.get(\"{}\").and_then(|v| v.as_list()).into_iter().flatten().filter_map(|v| v.as_bool())",
                        struct_ref, prop_name
                    )
            }
            PropertyType::String | PropertyType::DateTime => {
                format!(
                        "{}.props.get(\"{}\").and_then(|v| v.as_list()).into_iter().flatten().filter_map(|v| v.as_str()).map(|s| s.to_string())",
                        struct_ref, prop_name
                    )
            }
            PropertyType::ElementRef(_) => {
                format!(
                        "{}.props.get(\"{}\").and_then(|v| v.as_list()).into_iter().flatten().filter_map(|v| v.as_ref()).cloned()",
                        struct_ref, prop_name
                    )
            }
            PropertyType::Any => {
                format!(
                    "{}.props.get(\"{}\").and_then(|v| v.as_list()).into_iter().flatten().cloned()",
                    struct_ref, prop_name
                )
            }
        },
    }
}

/// Escape doc comment text for Rust.
fn escape_doc_comment(text: &str) -> String {
    // Replace backticks with code spans, escape special chars
    let escaped = text.replace("\\", "\\\\").replace("*/", "* /"); // Avoid closing doc comments

    // Wrap each line properly
    escaped
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Generate the property accessors code.
pub fn generate_property_accessors(resolved: &HashMap<String, ResolvedShape>) -> String {
    let mut output = String::new();

    // File header
    output.push_str("// This file is automatically generated by sysml-codegen.\n");
    output.push_str("// Do not edit manually.\n");
    output.push_str("//\n");
    output.push_str(&format!(
        "// Generated from {} element types with typed property accessors.\n",
        resolved.len()
    ));
    output.push('\n');

    // Note: ElementId and Value are already imported in lib.rs
    // So we don't add use statements here
    output.push_str("// Types used: ElementId, Value (imported in lib.rs)\n");
    output.push('\n');

    // Sort shapes by name for consistent output
    let mut shapes: Vec<_> = resolved.values().collect();
    shapes.sort_by(|a, b| a.element_type.cmp(&b.element_type));

    // Generate accessor structs
    for shape in &shapes {
        generate_accessor_struct(&mut output, shape);
    }

    // Generate cast methods impl block on Element
    output.push_str("/// Extension methods for casting Element to typed accessors.\n");
    output.push_str("impl Element {\n");
    for shape in &shapes {
        generate_cast_method(&mut output, shape);
    }
    output.push_str("}\n");

    output
}

/// Generate an accessor struct for a single shape.
fn generate_accessor_struct(output: &mut String, shape: &ResolvedShape) {
    let struct_name = accessor_struct_name(&shape.element_type);

    // Struct doc comment
    if let Some(desc) = &shape.description {
        output.push_str(&format!("/// {}\n", escape_doc_comment(desc)));
    } else {
        output.push_str(&format!(
            "/// Typed property accessor for `{}`.\n",
            shape.element_type
        ));
    }
    output.push_str("///\n");
    output.push_str("/// Provides type-safe access to properties defined on this element type.\n");

    // Struct definition
    output.push_str("#[derive(Debug, Clone, Copy)]\n");
    output.push_str(&format!(
        "pub struct {}<'a>(pub &'a Element);\n\n",
        struct_name
    ));

    // Impl block with property accessors
    output.push_str(&format!("impl<'a> {}<'a> {{\n", struct_name));

    // Access to underlying element
    output.push_str("    /// Get the underlying element.\n");
    output.push_str("    #[inline]\n");
    output.push_str("    pub fn element(&self) -> &Element {\n");
    output.push_str("        self.0\n");
    output.push_str("    }\n\n");

    // Generate a method for each property
    for prop in &shape.properties {
        generate_property_method(output, prop);
    }

    output.push_str("}\n\n");
}

/// Generate a property accessor method.
fn generate_property_method(output: &mut String, prop: &PropertyInfo) {
    let method_name = to_snake_case(&prop.name);
    let return_type = property_return_type(prop);
    let body = property_accessor_body(prop, "self.0");

    // Doc comment
    if let Some(desc) = &prop.description {
        output.push_str(&format!("    /// {}\n", escape_doc_comment(desc)));
    } else {
        output.push_str(&format!("    /// Get the `{}` property.\n", prop.name));
    }
    output.push_str("    ///\n");

    // Add cardinality info to doc
    let cardinality_str = match prop.cardinality {
        Cardinality::ExactlyOne => "Exactly-one (required)",
        Cardinality::ZeroOrOne => "Zero-or-one (optional)",
        Cardinality::ZeroOrMany => "Zero-or-many",
        Cardinality::OneOrMany => "One-or-many",
    };
    output.push_str(&format!("    /// Cardinality: {}\n", cardinality_str));

    if prop.read_only {
        output.push_str("    /// (read-only)\n");
    }

    // Method signature and body
    output.push_str("    #[inline]\n");
    output.push_str(&format!(
        "    pub fn {}(&self) -> {} {{\n",
        method_name, return_type
    ));
    output.push_str(&format!("        {}\n", body));
    output.push_str("    }\n\n");
}

/// Generate a cast method for Element.
fn generate_cast_method(output: &mut String, shape: &ResolvedShape) {
    let method_name = cast_method_name(&shape.element_type);
    let struct_name = accessor_struct_name(&shape.element_type);

    output.push_str(&format!(
        "    /// Cast this element to a `{}` accessor if it has the matching kind.\n",
        struct_name
    ));
    output.push_str(&format!(
        "    pub fn {}(&self) -> Option<{}<'_>> {{\n",
        method_name, struct_name
    ));
    output.push_str(&format!(
        "        if self.kind == ElementKind::{} {{\n",
        shape.element_type
    ));
    output.push_str(&format!("            Some({}(self))\n", struct_name));
    output.push_str("        } else {\n");
    output.push_str("            None\n");
    output.push_str("        }\n");
    output.push_str("    }\n\n");
}

/// Generate only the property accessors for shapes that exist in ElementKind.
///
/// This filters out shapes that don't have a corresponding ElementKind variant.
pub fn generate_property_accessors_filtered(
    resolved: &HashMap<String, ResolvedShape>,
    valid_element_kinds: &[String],
) -> String {
    let valid_set: std::collections::HashSet<&str> =
        valid_element_kinds.iter().map(|s| s.as_str()).collect();

    let filtered: HashMap<String, ResolvedShape> = resolved
        .iter()
        .filter(|(k, _)| valid_set.contains(k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    generate_property_accessors(&filtered)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("owningType"), "owning_type");
        assert_eq!(to_snake_case("PartUsage"), "part_usage");
        assert_eq!(to_snake_case("isVariation"), "is_variation");
        assert_eq!(to_snake_case("XMLLiteral"), "xml_literal");
        assert_eq!(to_snake_case("elementId"), "element_id");
        assert_eq!(to_snake_case("type"), "type_");
    }

    #[test]
    fn test_accessor_struct_name() {
        assert_eq!(accessor_struct_name("PartUsage"), "PartUsageProps");
        assert_eq!(accessor_struct_name("Element"), "ElementProps");
    }

    #[test]
    fn test_cast_method_name() {
        assert_eq!(cast_method_name("PartUsage"), "as_part_usage");
        assert_eq!(cast_method_name("Element"), "as_element");
    }

    #[test]
    fn test_property_return_type() {
        let bool_prop = PropertyInfo {
            name: "isVariation".to_owned(),
            cardinality: Cardinality::ExactlyOne,
            property_type: PropertyType::Bool,
            read_only: false,
            description: None,
        };
        assert_eq!(property_return_type(&bool_prop), "bool");

        let optional_ref = PropertyInfo {
            name: "owningType".to_owned(),
            cardinality: Cardinality::ZeroOrOne,
            property_type: PropertyType::ElementRef("Type".to_owned()),
            read_only: false,
            description: None,
        };
        assert_eq!(
            property_return_type(&optional_ref),
            "Option<ElementId /* Type */>"
        );

        let many_refs = PropertyInfo {
            name: "ownedElement".to_owned(),
            cardinality: Cardinality::ZeroOrMany,
            property_type: PropertyType::ElementRef("Element".to_owned()),
            read_only: false,
            description: None,
        };
        assert_eq!(
            property_return_type(&many_refs),
            "impl Iterator<Item = ElementId> + '_"
        );
    }

    #[test]
    fn test_generate_minimal() {
        use crate::shapes_parser::Cardinality;

        let mut resolved = HashMap::new();
        resolved.insert(
            "TestType".to_owned(),
            ResolvedShape {
                element_type: "TestType".to_owned(),
                properties: vec![
                    PropertyInfo {
                        name: "testBool".to_owned(),
                        cardinality: Cardinality::ExactlyOne,
                        property_type: PropertyType::Bool,
                        read_only: false,
                        description: Some("A test boolean.".to_owned()),
                    },
                    PropertyInfo {
                        name: "testRef".to_owned(),
                        cardinality: Cardinality::ZeroOrOne,
                        property_type: PropertyType::ElementRef("Other".to_owned()),
                        read_only: false,
                        description: None,
                    },
                ],
                supertypes: vec![],
                description: Some("A test type for unit testing.".to_owned()),
            },
        );

        let code = generate_property_accessors(&resolved);

        // Check struct is generated
        assert!(code.contains("pub struct TestTypeProps<'a>"));

        // Check methods are generated
        assert!(code.contains("pub fn test_bool(&self) -> bool"));
        assert!(code.contains("pub fn test_ref(&self) -> Option<ElementId /* Other */>"));

        // Check cast method
        assert!(code.contains("pub fn as_test_type(&self) -> Option<TestTypeProps<'_>>"));
    }
}
