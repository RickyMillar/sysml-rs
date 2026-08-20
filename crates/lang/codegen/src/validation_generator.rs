//! Validation code generator.
//!
//! This module generates validation methods for property accessors
//! based on shape constraints.

use crate::inheritance::ResolvedShape;
use crate::shapes_parser::{Cardinality, PropertyInfo, PropertyType};
use std::collections::HashMap;

/// Convert a type name to the accessor struct name.
fn accessor_struct_name(element_type: &str) -> String {
    format!("{}Props", element_type)
}

/// Get the expected type name for error messages.
fn expected_type_name(prop_type: &PropertyType) -> &'static str {
    match prop_type {
        PropertyType::Bool => "bool",
        PropertyType::String => "string",
        PropertyType::DateTime => "datetime",
        PropertyType::ElementRef(_) => "ElementId",
        PropertyType::Any => "any",
    }
}

/// Generate validation methods for all shapes.
pub fn generate_validation_methods(resolved: &HashMap<String, ResolvedShape>) -> String {
    let mut output = String::new();

    // Sort shapes for consistent output
    let mut shapes: Vec<_> = resolved.values().collect();
    shapes.sort_by(|a, b| a.element_type.cmp(&b.element_type));

    for shape in shapes {
        generate_validation_impl(&mut output, shape);
    }

    output
}

/// Generate validation impl for a single shape.
fn generate_validation_impl(output: &mut String, shape: &ResolvedShape) {
    let struct_name = accessor_struct_name(&shape.element_type);

    output.push_str(&format!("impl<'a> {}<'a> {{\n", struct_name));
    output.push_str("    /// Validate this element against its shape constraints.\n");
    output.push_str("    ///\n");
    output.push_str("    /// Returns a list of validation errors, empty if valid.\n");
    output.push_str("    pub fn validate(&self) -> ValidationResult {\n");
    output.push_str("        let mut result = ValidationResult::new();\n\n");

    // Generate validation for each property
    for prop in &shape.properties {
        generate_property_validation(output, prop);
    }

    output.push_str("        result\n");
    output.push_str("    }\n");
    output.push_str("}\n\n");
}

/// Generate validation code for a single property.
fn generate_property_validation(output: &mut String, prop: &PropertyInfo) {
    let prop_name = &prop.name;

    match prop.cardinality {
        Cardinality::ExactlyOne => {
            // Required property - check it exists and has correct type
            if matches!(prop.property_type, PropertyType::Bool) {
                // Booleans are always present (default to false), no validation needed
                output.push_str(&format!(
                    "        // {} is a boolean, always present\n\n",
                    prop_name
                ));
            } else {
                output.push_str(&format!(
                    "        // Check required property: {}\n",
                    prop_name
                ));
                output.push_str(&format!(
                    "        if self.0.props.get(\"{}\").is_none() {{\n",
                    prop_name
                ));
                output.push_str(&format!(
                    "            result.add_error(ValidationError::missing_required(\"{}\"));\n",
                    prop_name
                ));
                output.push_str("        }\n");

                // Type check
                generate_type_check(output, prop);

                // MaxCardinality check - exactly one means at most 1 value
                generate_max_cardinality_check(output, prop_name);
                output.push('\n');
            }
        }
        Cardinality::ZeroOrOne => {
            // Optional property - check type if present
            output.push_str(&format!(
                "        // Check optional property type: {}\n",
                prop_name
            ));
            generate_type_check(output, prop);

            // MaxCardinality check - zero or one means at most 1 value
            generate_max_cardinality_check(output, prop_name);
            output.push('\n');
        }
        Cardinality::OneOrMany => {
            // Must have at least one value
            output.push_str(&format!(
                "        // Check one-or-many property: {}\n",
                prop_name
            ));
            output.push_str(&format!(
                "        if let Some(v) = self.0.props.get(\"{}\") {{\n",
                prop_name
            ));
            output.push_str("            if let Some(list) = v.as_list() {\n");
            output.push_str("                if list.is_empty() {\n");
            output.push_str(&format!(
                "                    result.add_error(ValidationError::min_cardinality(\"{}\"));\n",
                prop_name
            ));
            output.push_str("                }\n");
            output.push_str("            } else {\n");
            output.push_str("                // Single value is acceptable\n");
            output.push_str("            }\n");
            output.push_str("        } else {\n");
            output.push_str(&format!(
                "            result.add_error(ValidationError::min_cardinality(\"{}\"));\n",
                prop_name
            ));
            output.push_str("        }\n\n");
        }
        Cardinality::ZeroOrMany => {
            // No cardinality constraints, just type check
            output.push_str(&format!(
                "        // Check zero-or-many property type: {}\n",
                prop_name
            ));
            generate_list_type_check(output, prop);
            output.push('\n');
        }
    }
}

/// Generate type check code for a property value.
fn generate_type_check(output: &mut String, prop: &PropertyInfo) {
    let prop_name = &prop.name;
    let expected = expected_type_name(&prop.property_type);

    output.push_str(&format!(
        "        if let Some(v) = self.0.props.get(\"{}\") {{\n",
        prop_name
    ));

    match &prop.property_type {
        PropertyType::Bool => {
            output.push_str("            if v.as_bool().is_none() && !v.is_null() {\n");
        }
        PropertyType::String | PropertyType::DateTime => {
            output.push_str("            if v.as_str().is_none() && !v.is_null() {\n");
        }
        PropertyType::ElementRef(_) => {
            output.push_str("            if v.as_ref().is_none() && !v.is_null() {\n");
        }
        PropertyType::Any => {
            // Any type is valid
            output.push_str("            if false { // Any type is valid\n");
        }
    }

    output.push_str(&format!(
        "                result.add_error(ValidationError::wrong_type(\"{}\", \"{}\", v.type_name()));\n",
        prop_name, expected
    ));
    output.push_str("            }\n");
    output.push_str("        }\n");
}

/// Generate type check for list property values.
fn generate_list_type_check(output: &mut String, prop: &PropertyInfo) {
    let prop_name = &prop.name;
    let expected = expected_type_name(&prop.property_type);

    output.push_str(&format!(
        "        if let Some(v) = self.0.props.get(\"{}\") {{\n",
        prop_name
    ));
    output.push_str("            if let Some(list) = v.as_list() {\n");
    output.push_str("                for item in list {\n");

    match &prop.property_type {
        PropertyType::Bool => {
            output
                .push_str("                    if item.as_bool().is_none() && !item.is_null() {\n");
        }
        PropertyType::String | PropertyType::DateTime => {
            output
                .push_str("                    if item.as_str().is_none() && !item.is_null() {\n");
        }
        PropertyType::ElementRef(_) => {
            output
                .push_str("                    if item.as_ref().is_none() && !item.is_null() {\n");
        }
        PropertyType::Any => {
            output.push_str("                    if false { // Any type is valid\n");
        }
    }

    output.push_str(&format!(
        "                        result.add_error(ValidationError::wrong_type(\"{}\", \"{}\", item.type_name()));\n",
        prop_name, expected
    ));
    output.push_str("                        break; // Report only first type error\n");
    output.push_str("                    }\n");
    output.push_str("                }\n");
    output.push_str("            }\n");
    output.push_str("        }\n");
}

/// Generate MaxCardinality check code for ZeroOrOne and ExactlyOne properties.
///
/// Validates that if the property value is a list, it has at most 1 element.
fn generate_max_cardinality_check(output: &mut String, prop_name: &str) {
    output.push_str(&format!(
        "        if let Some(crate::meta::Value::List(list)) = self.0.props.get(\"{}\") {{\n",
        prop_name
    ));
    output.push_str("            if list.len() > 1 {\n");
    output.push_str(&format!(
        "                result.add_error(ValidationError::max_cardinality(\"{}\"));\n",
        prop_name
    ));
    output.push_str("            }\n");
    output.push_str("        }\n");
}

/// Generate validation methods only for shapes that exist in ElementKind.
pub fn generate_validation_methods_filtered(
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

    generate_validation_methods(&filtered)
}

/// Generate a `validate_element_properties` dispatcher function.
///
/// This generates a function that dispatches to the correct per-type
/// `validate()` method based on the element's `ElementKind`. The generated
/// function has the signature:
///
/// ```ignore
/// pub fn validate_element_properties(element: &Element) -> ValidationResult
/// ```
///
/// This is Phase A scaffolding -- it calls the already-generated per-type
/// `validate()` methods that exist on every `XxxProps` struct.
pub fn generate_property_validation_dispatcher(
    resolved: &HashMap<String, ResolvedShape>,
) -> String {
    let mut output = String::new();

    output.push_str("\n// === Property Validation Dispatcher ===\n");
    output.push_str(
        "// Generated by validation_generator::generate_property_validation_dispatcher\n\n",
    );

    output.push_str("/// Validate an element's properties against its shape constraints.\n");
    output.push_str("///\n");
    output.push_str(
        "/// Dispatches to the per-type `validate()` method based on the element's kind.\n",
    );
    output.push_str(
        "/// Returns a `ValidationResult` containing any property constraint violations.\n",
    );
    output.push_str("///\n");
    output.push_str("/// # Examples\n");
    output.push_str("///\n");
    output.push_str("/// ```ignore\n");
    output.push_str("/// let result = validate_element_properties(&element);\n");
    output.push_str("/// if !result.is_valid() {\n");
    output.push_str("///     for error in &result.errors {\n");
    output.push_str("///         eprintln!(\"Property error: {}\", error);\n");
    output.push_str("///     }\n");
    output.push_str("/// }\n");
    output.push_str("/// ```\n");
    output
        .push_str("pub fn validate_element_properties(element: &Element) -> ValidationResult {\n");
    output.push_str("    match element.kind {\n");

    // Sort shapes for consistent output
    let mut shapes: Vec<_> = resolved.values().collect();
    shapes.sort_by(|a, b| a.element_type.cmp(&b.element_type));

    for shape in &shapes {
        let struct_name = accessor_struct_name(&shape.element_type);
        output.push_str(&format!(
            "        ElementKind::{} => {}(element).validate(),\n",
            shape.element_type, struct_name
        ));
    }

    output.push_str("        _ => ValidationResult::new(),\n");
    output.push_str("    }\n");
    output.push_str("}\n");

    output
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::shapes_parser::Cardinality;

    fn make_prop(name: &str, cardinality: Cardinality, prop_type: PropertyType) -> PropertyInfo {
        PropertyInfo {
            name: name.to_string(),
            cardinality,
            property_type: prop_type,
            read_only: false,
            description: None,
        }
    }

    fn make_shape(element_type: &str, properties: Vec<PropertyInfo>) -> ResolvedShape {
        ResolvedShape {
            element_type: element_type.to_string(),
            properties,
            supertypes: vec![],
            description: None,
        }
    }

    #[test]
    fn test_generate_validation() {
        let mut resolved = HashMap::new();
        resolved.insert(
            "TestType".to_owned(),
            make_shape(
                "TestType",
                vec![
                    make_prop(
                        "requiredProp",
                        Cardinality::ExactlyOne,
                        PropertyType::ElementRef("Type".to_owned()),
                    ),
                    make_prop("optionalProp", Cardinality::ZeroOrOne, PropertyType::String),
                ],
            ),
        );

        let code = generate_validation_methods(&resolved);

        // Check method is generated
        assert!(code.contains("impl<'a> TestTypeProps<'a>"));
        assert!(code.contains("pub fn validate(&self) -> ValidationResult"));

        // Check required property validation
        assert!(code.contains("missing_required(\"requiredProp\")"));

        // Check optional property type check
        assert!(code.contains("Check optional property type: optionalProp"));
    }

    #[test]
    fn required_property_generates_missing_check() {
        let mut resolved = HashMap::new();
        resolved.insert(
            "Foo".to_owned(),
            make_shape(
                "Foo",
                vec![make_prop(
                    "name",
                    Cardinality::ExactlyOne,
                    PropertyType::String,
                )],
            ),
        );
        let code = generate_validation_methods(&resolved);
        assert!(
            code.contains("missing_required(\"name\")"),
            "should generate missing_required check for ExactlyOne string property"
        );
    }

    #[test]
    fn boolean_required_property_skips_missing_check() {
        let mut resolved = HashMap::new();
        resolved.insert(
            "Foo".to_owned(),
            make_shape(
                "Foo",
                vec![make_prop(
                    "isAbstract",
                    Cardinality::ExactlyOne,
                    PropertyType::Bool,
                )],
            ),
        );
        let code = generate_validation_methods(&resolved);
        assert!(
            !code.contains("missing_required(\"isAbstract\")"),
            "boolean properties default to false, no missing check needed"
        );
        assert!(code.contains("isAbstract is a boolean, always present"));
    }

    #[test]
    fn one_or_many_generates_min_cardinality_check() {
        let mut resolved = HashMap::new();
        resolved.insert(
            "Bar".to_owned(),
            make_shape(
                "Bar",
                vec![make_prop(
                    "members",
                    Cardinality::OneOrMany,
                    PropertyType::ElementRef("Element".to_owned()),
                )],
            ),
        );
        let code = generate_validation_methods(&resolved);
        assert!(
            code.contains("min_cardinality(\"members\")"),
            "OneOrMany should generate min cardinality check"
        );
    }

    #[test]
    fn empty_type_generates_empty_validate() {
        let mut resolved = HashMap::new();
        resolved.insert("Empty".to_owned(), make_shape("Empty", vec![]));
        let code = generate_validation_methods(&resolved);
        assert!(code.contains("impl<'a> EmptyProps<'a>"));
        assert!(code.contains("pub fn validate(&self) -> ValidationResult"));
        // The body should just be creating result and returning it
        assert!(code.contains("let mut result = ValidationResult::new();"));
        assert!(code.contains("result\n    }\n"));
    }

    #[test]
    fn max_cardinality_check_for_exactly_one() {
        let mut resolved = HashMap::new();
        resolved.insert(
            "Baz".to_owned(),
            make_shape(
                "Baz",
                vec![make_prop(
                    "target",
                    Cardinality::ExactlyOne,
                    PropertyType::ElementRef("Type".to_owned()),
                )],
            ),
        );
        let code = generate_validation_methods(&resolved);
        assert!(
            code.contains("max_cardinality(\"target\")"),
            "ExactlyOne should generate max cardinality check"
        );
    }

    #[test]
    fn zero_or_one_generates_max_cardinality_check() {
        let mut resolved = HashMap::new();
        resolved.insert(
            "Qux".to_owned(),
            make_shape(
                "Qux",
                vec![make_prop(
                    "owner",
                    Cardinality::ZeroOrOne,
                    PropertyType::ElementRef("Namespace".to_owned()),
                )],
            ),
        );
        let code = generate_validation_methods(&resolved);
        assert!(
            code.contains("max_cardinality(\"owner\")"),
            "ZeroOrOne should generate max cardinality check"
        );
    }

    #[test]
    fn zero_or_many_generates_list_type_check() {
        let mut resolved = HashMap::new();
        resolved.insert(
            "Multi".to_owned(),
            make_shape(
                "Multi",
                vec![make_prop(
                    "items",
                    Cardinality::ZeroOrMany,
                    PropertyType::String,
                )],
            ),
        );
        let code = generate_validation_methods(&resolved);
        assert!(
            code.contains("Check zero-or-many property type: items"),
            "ZeroOrMany should get a list type check"
        );
        // Should NOT have missing_required or min_cardinality
        assert!(!code.contains("missing_required(\"items\")"));
        assert!(!code.contains("min_cardinality(\"items\")"));
    }

    #[test]
    fn filtered_generation_excludes_non_element_kinds() {
        let mut resolved = HashMap::new();
        resolved.insert(
            "Good".to_owned(),
            make_shape(
                "Good",
                vec![make_prop(
                    "x",
                    Cardinality::ExactlyOne,
                    PropertyType::String,
                )],
            ),
        );
        resolved.insert(
            "NotAnElement".to_owned(),
            make_shape(
                "NotAnElement",
                vec![make_prop("y", Cardinality::ExactlyOne, PropertyType::Bool)],
            ),
        );

        let valid_kinds = vec!["Good".to_owned()];
        let code = generate_validation_methods_filtered(&resolved, &valid_kinds);

        assert!(code.contains("GoodProps"), "Good should be included");
        assert!(
            !code.contains("NotAnElementProps"),
            "NotAnElement should be filtered out"
        );
    }

    #[test]
    fn dispatcher_generated_for_all_types() {
        let mut resolved = HashMap::new();
        resolved.insert(
            "PartUsage".to_owned(),
            make_shape(
                "PartUsage",
                vec![make_prop(
                    "definition",
                    Cardinality::ZeroOrOne,
                    PropertyType::ElementRef("PartDefinition".to_owned()),
                )],
            ),
        );
        resolved.insert(
            "Element".to_owned(),
            make_shape(
                "Element",
                vec![make_prop(
                    "declaredName",
                    Cardinality::ZeroOrOne,
                    PropertyType::String,
                )],
            ),
        );

        let code = generate_property_validation_dispatcher(&resolved);

        // Check function signature
        assert!(
            code.contains(
                "pub fn validate_element_properties(element: &Element) -> ValidationResult"
            ),
            "dispatcher function signature"
        );

        // Check dispatch arms for both types
        assert!(
            code.contains("ElementKind::PartUsage => PartUsageProps(element).validate()"),
            "PartUsage dispatch arm"
        );
        assert!(
            code.contains("ElementKind::Element => ElementProps(element).validate()"),
            "Element dispatch arm"
        );

        // Check wildcard fallback
        assert!(
            code.contains("_ => ValidationResult::new()"),
            "wildcard arm returns empty result"
        );
    }

    #[test]
    fn dispatcher_with_empty_shapes() {
        let resolved = HashMap::new();
        let code = generate_property_validation_dispatcher(&resolved);

        assert!(code.contains("pub fn validate_element_properties"));
        assert!(code.contains("_ => ValidationResult::new()"));
    }
}
