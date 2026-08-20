//! OSLC Shapes parser for SysML/KerML property definitions.
//!
//! This module parses OSLC shapes TTL files to extract property constraints
//! for code generation of typed property accessors.

use std::collections::HashMap;

/// Cardinality constraint from OSLC shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cardinality {
    /// `oslc:Zero-or-many` → `impl Iterator<Item=T>`
    ZeroOrMany,
    /// `oslc:Zero-or-one` → `Option<T>`
    #[default]
    ZeroOrOne,
    /// `oslc:Exactly-one` → `T` (required)
    ExactlyOne,
    /// `oslc:One-or-many` → `impl Iterator<Item=T>` (rare, ~9 uses)
    OneOrMany,
}

/// Property type from `oslc:range`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PropertyType {
    /// Boolean property (no range, exactly-one cardinality).
    Bool,
    /// Reference to another element type.
    ElementRef(String),
    /// String value type.
    String,
    /// DateTime value type.
    DateTime,
    /// `oslc:Any` (rare).
    #[default]
    Any,
}

/// A single property constraint from shapes.
#[derive(Debug, Clone)]
pub struct PropertyInfo {
    /// The property name (e.g., "owningType").
    pub name: String,
    /// Cardinality constraint.
    pub cardinality: Cardinality,
    /// Property value type.
    pub property_type: PropertyType,
    /// Whether the property is read-only.
    pub read_only: bool,
    /// Documentation for the property.
    pub description: Option<String>,
}

/// A shape definition for one element type.
#[derive(Debug, Clone)]
pub struct ShapeInfo {
    /// The element type name (e.g., "PartUsage").
    pub element_type: String,
    /// The shape name (e.g., "PartUsageShape").
    pub shape_name: String,
    /// Properties defined directly on this shape.
    pub properties: Vec<PropertyInfo>,
    /// References to shared properties (e.g., ":aliasIds").
    pub property_refs: Vec<String>,
    /// Documentation for this shape.
    pub description: Option<String>,
}

/// Error type for shapes parsing failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// Invalid TTL syntax.
    #[error("invalid shapes syntax: {0}")]
    InvalidSyntax(String),
    /// Missing required field.
    #[error("missing field: {0}")]
    MissingField(String),
}

/// Parse OSLC shapes from TTL content.
///
/// Returns a tuple of (shapes, shared_properties).
pub fn parse_oslc_shapes(
    content: &str,
) -> Result<(Vec<ShapeInfo>, HashMap<String, PropertyInfo>), ParseError> {
    let mut shapes = Vec::new();
    let mut shared_properties: HashMap<String, PropertyInfo> = HashMap::new();

    // First pass: collect shared property definitions at the file level
    // These are standalone property definitions like `:aliasIds a oslc:Property ; ...`
    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Skip whitespace and comments
        while i < len && (chars[i].is_whitespace() || chars[i] == '#') {
            if chars[i] == '#' {
                // Skip to end of line
                while i < len && chars[i] != '\n' {
                    i += 1;
                }
            }
            i += 1;
        }

        if i >= len {
            break;
        }

        // Check for @prefix declarations (skip them)
        if starts_with_at(&chars, i, "@prefix") {
            while i < len && chars[i] != '.' {
                i += 1;
            }
            i += 1; // skip the '.'
            continue;
        }

        // Look for resource definitions
        if chars[i] == ':' || (chars[i].is_alphabetic() && !starts_with_at(&chars, i, "@")) {
            // Parse the subject (resource name)
            let subject_start = i;
            while i < len && !chars[i].is_whitespace() {
                i += 1;
            }
            let subject: String = chars[subject_start..i].iter().collect();

            // Skip whitespace
            while i < len && chars[i].is_whitespace() {
                i += 1;
            }

            // Check what type of definition this is
            if i < len && chars[i] == 'a' && i + 1 < len && chars[i + 1].is_whitespace() {
                i += 2; // skip 'a '
                while i < len && chars[i].is_whitespace() {
                    i += 1;
                }

                // Read the type
                let type_start = i;
                while i < len && !chars[i].is_whitespace() && chars[i] != ';' && chars[i] != '.' {
                    i += 1;
                }
                let rdf_type: String = chars[type_start..i].iter().collect();

                if rdf_type == "oslc:ResourceShape" {
                    // Parse a shape definition
                    let shape = parse_shape_block(&chars, &mut i, &subject)?;
                    shapes.push(shape);
                } else if rdf_type == "oslc:Property" {
                    // Parse a shared property definition
                    let prop = parse_property_definition(&chars, &mut i)?;
                    let prop_name = extract_local_name(&subject);
                    shared_properties.insert(prop_name, prop);
                } else if rdf_type == "oslc:ResourceShapeConstraints" {
                    // Skip the header block
                    skip_to_end_of_block(&chars, &mut i);
                }
            } else {
                // Skip to end of this statement
                skip_to_end_of_block(&chars, &mut i);
            }
        } else {
            i += 1;
        }
    }

    Ok((shapes, shared_properties))
}

/// Check if the characters starting at position `i` match the given string.
fn starts_with_at(chars: &[char], i: usize, s: &str) -> bool {
    let s_chars: Vec<char> = s.chars().collect();
    if i + s_chars.len() > chars.len() {
        return false;
    }
    for (j, c) in s_chars.iter().enumerate() {
        if chars[i + j] != *c {
            return false;
        }
    }
    true
}

/// Extract the local name from a prefixed name (e.g., ":aliasIds" -> "aliasIds").
fn extract_local_name(prefixed: &str) -> String {
    if let Some(idx) = prefixed.rfind(':') {
        prefixed[idx + 1..].to_owned()
    } else {
        prefixed.to_owned()
    }
}

/// Skip to the end of a block (terminated by '.' at the same nesting level).
fn skip_to_end_of_block(chars: &[char], i: &mut usize) {
    let mut bracket_depth = 0;
    let len = chars.len();

    while *i < len {
        let c = chars[*i];
        if c == '[' {
            bracket_depth += 1;
        } else if c == ']' {
            bracket_depth -= 1;
        } else if c == '.' && bracket_depth == 0 {
            *i += 1;
            return;
        } else if c == '"' {
            // Skip quoted strings
            *i += 1;
            // Check for triple quotes
            if *i + 1 < len && chars[*i] == '"' && chars[*i + 1] == '"' {
                *i += 2;
                // Skip until closing triple quotes
                while *i + 2 < len {
                    if chars[*i] == '"' && chars[*i + 1] == '"' && chars[*i + 2] == '"' {
                        *i += 3;
                        break;
                    }
                    *i += 1;
                }
            } else {
                // Skip until closing quote
                while *i < len && chars[*i] != '"' {
                    if chars[*i] == '\\' && *i + 1 < len {
                        *i += 2;
                    } else {
                        *i += 1;
                    }
                }
                if *i < len {
                    *i += 1; // skip closing quote
                }
            }
            continue;
        }
        *i += 1;
    }
}

/// Parse a ResourceShape block.
fn parse_shape_block(
    chars: &[char],
    i: &mut usize,
    subject: &str,
) -> Result<ShapeInfo, ParseError> {
    let shape_name = extract_local_name(subject);
    let mut element_type = String::new();
    let mut properties = Vec::new();
    let mut property_refs = Vec::new();
    let mut description = None;

    let len = chars.len();

    // Skip whitespace and ';' after type declaration
    while *i < len && (chars[*i].is_whitespace() || chars[*i] == ';') {
        *i += 1;
    }

    // Parse predicates until we hit '.'
    while *i < len && chars[*i] != '.' {
        // Skip whitespace
        while *i < len && chars[*i].is_whitespace() {
            *i += 1;
        }

        if *i >= len || chars[*i] == '.' {
            break;
        }

        // Read predicate
        let pred_start = *i;
        while *i < len
            && !chars[*i].is_whitespace()
            && chars[*i] != '.'
            && chars[*i] != ';'
            && chars[*i] != ','
        {
            *i += 1;
        }
        let predicate: String = chars[pred_start..*i].iter().collect();

        // Skip whitespace
        while *i < len && chars[*i].is_whitespace() {
            *i += 1;
        }

        match predicate.as_str() {
            "oslc:describes" => {
                // Read the element type
                let type_start = *i;
                while *i < len && !chars[*i].is_whitespace() && chars[*i] != ';' && chars[*i] != '.'
                {
                    *i += 1;
                }
                let full_type: String = chars[type_start..*i].iter().collect();
                element_type = extract_local_name(&full_type);
            }
            "oslc:property" => {
                // Parse property - can be inline [ ... ] or reference :name
                while *i < len && chars[*i].is_whitespace() {
                    *i += 1;
                }

                if *i < len && chars[*i] == '[' {
                    // Inline property definition
                    let prop = parse_inline_property(chars, i)?;
                    properties.push(prop);
                } else {
                    // Property reference
                    let ref_start = *i;
                    while *i < len
                        && !chars[*i].is_whitespace()
                        && chars[*i] != ';'
                        && chars[*i] != ','
                        && chars[*i] != '.'
                    {
                        *i += 1;
                    }
                    let prop_ref: String = chars[ref_start..*i].iter().collect();
                    property_refs.push(extract_local_name(&prop_ref));
                }

                // Handle multiple properties separated by ','
                while *i < len && chars[*i].is_whitespace() {
                    *i += 1;
                }
                while *i < len && chars[*i] == ',' {
                    *i += 1;
                    while *i < len && chars[*i].is_whitespace() {
                        *i += 1;
                    }

                    if *i < len && chars[*i] == '[' {
                        let prop = parse_inline_property(chars, i)?;
                        properties.push(prop);
                    } else if *i < len && chars[*i] != ';' && chars[*i] != '.' {
                        let ref_start = *i;
                        while *i < len
                            && !chars[*i].is_whitespace()
                            && chars[*i] != ';'
                            && chars[*i] != ','
                            && chars[*i] != '.'
                        {
                            *i += 1;
                        }
                        let prop_ref: String = chars[ref_start..*i].iter().collect();
                        property_refs.push(extract_local_name(&prop_ref));
                    }

                    while *i < len && chars[*i].is_whitespace() {
                        *i += 1;
                    }
                }
            }
            "dcterms:description" => {
                description = Some(parse_quoted_string(chars, i)?);
            }
            "dcterms:title" => {
                // Skip the title
                parse_quoted_string(chars, i)?;
            }
            _ => {
                // Skip unknown predicate's object
                skip_object(chars, i);
            }
        }

        // Skip whitespace
        while *i < len && chars[*i].is_whitespace() {
            *i += 1;
        }

        // Skip ';' if present
        if *i < len && chars[*i] == ';' {
            *i += 1;
        }
    }

    // Skip the final '.'
    if *i < len && chars[*i] == '.' {
        *i += 1;
    }

    if element_type.is_empty() {
        // Try to derive from shape name
        if shape_name.ends_with("Shape") {
            element_type = shape_name[..shape_name.len() - 5].to_string();
        }
    }

    Ok(ShapeInfo {
        element_type,
        shape_name,
        properties,
        property_refs,
        description,
    })
}

/// Parse an inline property definition `[ a oslc:Property ; ... ]`.
fn parse_inline_property(chars: &[char], i: &mut usize) -> Result<PropertyInfo, ParseError> {
    let len = chars.len();

    // Skip opening '['
    if *i < len && chars[*i] == '[' {
        *i += 1;
    }

    // Skip whitespace and 'a oslc:Property ;'
    while *i < len && chars[*i].is_whitespace() {
        *i += 1;
    }

    // Skip 'a oslc:Property ;'
    if *i < len && chars[*i] == 'a' {
        *i += 1;
        while *i < len && chars[*i].is_whitespace() {
            *i += 1;
        }
        // Skip 'oslc:Property'
        while *i < len && !chars[*i].is_whitespace() && chars[*i] != ';' {
            *i += 1;
        }
        // Skip ';'
        while *i < len && (chars[*i].is_whitespace() || chars[*i] == ';') {
            *i += 1;
        }
    }

    parse_property_definition(chars, i)
}

/// Parse property attributes from current position.
fn parse_property_definition(chars: &[char], i: &mut usize) -> Result<PropertyInfo, ParseError> {
    let len = chars.len();
    let mut name = String::new();
    let mut cardinality = Cardinality::ZeroOrOne;
    let mut property_type = PropertyType::Any;
    let mut read_only = false;
    let mut description = None;

    // Parse predicates until ']' or '.'
    while *i < len && chars[*i] != ']' && chars[*i] != '.' {
        // Skip whitespace
        while *i < len && chars[*i].is_whitespace() {
            *i += 1;
        }

        if *i >= len || chars[*i] == ']' || chars[*i] == '.' {
            break;
        }

        // Read predicate
        let pred_start = *i;
        while *i < len
            && !chars[*i].is_whitespace()
            && chars[*i] != ']'
            && chars[*i] != ';'
            && chars[*i] != ','
        {
            *i += 1;
        }
        let predicate: String = chars[pred_start..*i].iter().collect();

        // Skip whitespace
        while *i < len && chars[*i].is_whitespace() {
            *i += 1;
        }

        match predicate.as_str() {
            "oslc:name" => {
                name = parse_quoted_string(chars, i)?;
            }
            "oslc:occurs" => {
                let value_start = *i;
                while *i < len && !chars[*i].is_whitespace() && chars[*i] != ';' && chars[*i] != ']'
                {
                    *i += 1;
                }
                let value: String = chars[value_start..*i].iter().collect();
                cardinality = match value.as_str() {
                    "oslc:Zero-or-many" => Cardinality::ZeroOrMany,
                    "oslc:Zero-or-one" => Cardinality::ZeroOrOne,
                    "oslc:Exactly-one" => Cardinality::ExactlyOne,
                    "oslc:One-or-many" => Cardinality::OneOrMany,
                    _ => Cardinality::ZeroOrOne,
                };
            }
            "oslc:range" => {
                let value_start = *i;
                while *i < len && !chars[*i].is_whitespace() && chars[*i] != ';' && chars[*i] != ']'
                {
                    *i += 1;
                }
                let value: String = chars[value_start..*i].iter().collect();
                property_type = parse_property_type(&value);
            }
            "oslc:readOnly" => {
                let value_start = *i;
                while *i < len && !chars[*i].is_whitespace() && chars[*i] != ';' && chars[*i] != ']'
                {
                    *i += 1;
                }
                let value: String = chars[value_start..*i].iter().collect();
                read_only = value == "true";
            }
            "oslc:valueType" => {
                let value_start = *i;
                while *i < len && !chars[*i].is_whitespace() && chars[*i] != ';' && chars[*i] != ']'
                {
                    *i += 1;
                }
                let value: String = chars[value_start..*i].iter().collect();
                // Use value type to refine property type if range wasn't set
                if property_type == PropertyType::Any {
                    if value.contains("xsd:string") {
                        property_type = PropertyType::String;
                    } else if value.contains("xsd:dateTime") {
                        property_type = PropertyType::DateTime;
                    } else if value.contains("xsd:boolean") {
                        property_type = PropertyType::Bool;
                    }
                }
            }
            "dcterms:description" => {
                description = Some(parse_quoted_string(chars, i)?);
            }
            _ => {
                // Skip unknown predicate's object
                skip_object(chars, i);
            }
        }

        // Skip whitespace
        while *i < len && chars[*i].is_whitespace() {
            *i += 1;
        }

        // Skip ';' if present
        if *i < len && chars[*i] == ';' {
            *i += 1;
        }
    }

    // Skip closing ']' if present
    if *i < len && chars[*i] == ']' {
        *i += 1;
    }

    // Infer boolean type for Exactly-one properties without a range
    if property_type == PropertyType::Any && cardinality == Cardinality::ExactlyOne {
        // This is likely a boolean property
        property_type = PropertyType::Bool;
    }

    Ok(PropertyInfo {
        name,
        cardinality,
        property_type,
        read_only,
        description,
    })
}

/// Parse the property type from an oslc:range value.
fn parse_property_type(range: &str) -> PropertyType {
    if range == "oslc:Any" {
        PropertyType::Any
    } else if range.contains("xsd:string") {
        PropertyType::String
    } else if range.contains("xsd:dateTime") {
        PropertyType::DateTime
    } else if range.contains("xsd:boolean") {
        PropertyType::Bool
    } else {
        // Extract the type name from prefixed form
        let type_name = extract_local_name(range);
        PropertyType::ElementRef(type_name)
    }
}

/// Parse a quoted string (handles both single and triple quotes).
fn parse_quoted_string(chars: &[char], i: &mut usize) -> Result<String, ParseError> {
    let len = chars.len();

    // Skip whitespace
    while *i < len && chars[*i].is_whitespace() {
        *i += 1;
    }

    if *i >= len || chars[*i] != '"' {
        return Err(ParseError::InvalidSyntax(
            "expected quoted string".to_owned(),
        ));
    }

    *i += 1; // skip opening quote

    // Check for triple quotes
    let triple = *i + 1 < len && chars[*i] == '"' && chars[*i + 1] == '"';
    if triple {
        *i += 2; // skip additional quotes
    }

    let mut result = String::new();

    if triple {
        // Read until closing triple quotes
        while *i + 2 < len {
            if chars[*i] == '"' && chars[*i + 1] == '"' && chars[*i + 2] == '"' {
                *i += 3;
                break;
            }
            result.push(chars[*i]);
            *i += 1;
        }
    } else {
        // Read until closing quote
        while *i < len && chars[*i] != '"' {
            if chars[*i] == '\\' && *i + 1 < len {
                // Handle escape sequences
                *i += 1;
                match chars[*i] {
                    'n' => result.push('\n'),
                    't' => result.push('\t'),
                    '"' => result.push('"'),
                    '\\' => result.push('\\'),
                    c => result.push(c),
                }
            } else {
                result.push(chars[*i]);
            }
            *i += 1;
        }
        if *i < len {
            *i += 1; // skip closing quote
        }
    }

    // Skip type annotation like ^^rdf:XMLLiteral
    while *i < len && chars[*i] == '^' {
        *i += 1;
    }
    while *i < len && !chars[*i].is_whitespace() && chars[*i] != ';' && chars[*i] != ']' {
        *i += 1;
    }

    // Strip HTML tags from description
    let cleaned = strip_html(&result);

    Ok(cleaned)
}

/// Skip an object value (handles nested brackets, quotes, etc.).
fn skip_object(chars: &[char], i: &mut usize) {
    let len = chars.len();

    // Skip whitespace
    while *i < len && chars[*i].is_whitespace() {
        *i += 1;
    }

    if *i >= len {
        return;
    }

    if chars[*i] == '[' {
        // Skip bracketed block
        let mut depth = 1;
        *i += 1;
        while *i < len && depth > 0 {
            if chars[*i] == '[' {
                depth += 1;
            } else if chars[*i] == ']' {
                depth -= 1;
            } else if chars[*i] == '"' {
                // Skip quoted string within brackets
                *i += 1;
                if *i + 1 < len && chars[*i] == '"' && chars[*i + 1] == '"' {
                    *i += 2;
                    while *i + 2 < len {
                        if chars[*i] == '"' && chars[*i + 1] == '"' && chars[*i + 2] == '"' {
                            *i += 3;
                            break;
                        }
                        *i += 1;
                    }
                    continue;
                }
                while *i < len && chars[*i] != '"' {
                    if chars[*i] == '\\' && *i + 1 < len {
                        *i += 2;
                    } else {
                        *i += 1;
                    }
                }
                if *i < len {
                    *i += 1;
                }
                continue;
            }
            *i += 1;
        }
    } else if chars[*i] == '"' {
        // Skip quoted string
        *i += 1;
        if *i + 1 < len && chars[*i] == '"' && chars[*i + 1] == '"' {
            *i += 2;
            while *i + 2 < len {
                if chars[*i] == '"' && chars[*i + 1] == '"' && chars[*i + 2] == '"' {
                    *i += 3;
                    break;
                }
                *i += 1;
            }
        } else {
            while *i < len && chars[*i] != '"' {
                if chars[*i] == '\\' && *i + 1 < len {
                    *i += 2;
                } else {
                    *i += 1;
                }
            }
            if *i < len {
                *i += 1;
            }
        }
        // Skip type annotation
        while *i < len && chars[*i] == '^' {
            *i += 1;
        }
        while *i < len && !chars[*i].is_whitespace() && chars[*i] != ';' && chars[*i] != ']' {
            *i += 1;
        }
    } else {
        // Skip until whitespace or delimiter
        while *i < len
            && !chars[*i].is_whitespace()
            && chars[*i] != ';'
            && chars[*i] != ']'
            && chars[*i] != ','
            && chars[*i] != '.'
        {
            *i += 1;
        }
    }
}

/// Strip HTML tags from a string.
fn strip_html(s: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    let mut last_was_space = false;

    for c in s.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            if c.is_whitespace() {
                if !last_was_space {
                    result.push(' ');
                    last_was_space = true;
                }
            } else {
                result.push(c);
                last_was_space = false;
            }
        }
    }

    result.trim().to_owned()
}

/// Merge shared properties into shapes based on property references.
pub fn resolve_shared_properties(
    shapes: &mut [ShapeInfo],
    shared_properties: &HashMap<String, PropertyInfo>,
) {
    for shape in shapes.iter_mut() {
        for prop_ref in &shape.property_refs {
            if let Some(prop) = shared_properties.get(prop_ref) {
                // Clone the shared property if not already present
                if !shape.properties.iter().any(|p| p.name == prop.name) {
                    shape.properties.push(prop.clone());
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_shape() {
        let ttl = r#"
:TestShape a oslc:ResourceShape ;
    oslc:describes oslc_kerml:Test ;
    oslc:property [ a oslc:Property ;
            oslc:name "testProp" ;
            oslc:occurs oslc:Zero-or-one ;
            oslc:range oslc_kerml:Type ;
            oslc:readOnly false ;
            dcterms:description "A test property."^^rdf:XMLLiteral ] ;
    dcterms:description "A test shape."^^rdf:XMLLiteral ;
    dcterms:title "TestShape"^^rdf:XMLLiteral .
"#;

        let (shapes, _) = parse_oslc_shapes(ttl).unwrap();
        assert_eq!(shapes.len(), 1);
        assert_eq!(shapes[0].element_type, "Test");
        assert_eq!(shapes[0].shape_name, "TestShape");
        assert_eq!(shapes[0].properties.len(), 1);
        assert_eq!(shapes[0].properties[0].name, "testProp");
        assert_eq!(shapes[0].properties[0].cardinality, Cardinality::ZeroOrOne);
        assert_eq!(
            shapes[0].properties[0].property_type,
            PropertyType::ElementRef("Type".to_owned())
        );
        assert!(!shapes[0].properties[0].read_only);
    }

    #[test]
    fn test_parse_multiple_properties() {
        let ttl = r#"
:MultiShape a oslc:ResourceShape ;
    oslc:describes oslc_sysml:Multi ;
    oslc:property [ a oslc:Property ;
            oslc:name "prop1" ;
            oslc:occurs oslc:Exactly-one ;
            dcterms:description "First prop." ],
        [ a oslc:Property ;
            oslc:name "prop2" ;
            oslc:occurs oslc:Zero-or-many ;
            oslc:range oslc_sysml:Element ] .
"#;

        let (shapes, _) = parse_oslc_shapes(ttl).unwrap();
        assert_eq!(shapes.len(), 1);
        assert_eq!(shapes[0].properties.len(), 2);
        assert_eq!(shapes[0].properties[0].name, "prop1");
        assert_eq!(shapes[0].properties[0].cardinality, Cardinality::ExactlyOne);
        // Exactly-one with no range is inferred as Bool
        assert_eq!(shapes[0].properties[0].property_type, PropertyType::Bool);
        assert_eq!(shapes[0].properties[1].name, "prop2");
        assert_eq!(shapes[0].properties[1].cardinality, Cardinality::ZeroOrMany);
    }

    #[test]
    fn test_parse_property_references() {
        let ttl = r#"
:RefShape a oslc:ResourceShape ;
    oslc:describes oslc_kerml:Ref ;
    oslc:property :aliasIds,
        :elementId .

:aliasIds a oslc:Property ;
    oslc:name "aliasIds" ;
    oslc:occurs oslc:Zero-or-many ;
    dcterms:description "Alias IDs."^^rdf:XMLLiteral .

:elementId a oslc:Property ;
    oslc:name "elementId" ;
    oslc:occurs oslc:Exactly-one ;
    dcterms:description "Element ID."^^rdf:XMLLiteral .
"#;

        let (mut shapes, shared) = parse_oslc_shapes(ttl).unwrap();
        assert_eq!(shapes.len(), 1);
        assert_eq!(shapes[0].property_refs.len(), 2);
        assert!(shapes[0].property_refs.contains(&"aliasIds".to_owned()));
        assert!(shapes[0].property_refs.contains(&"elementId".to_owned()));

        assert_eq!(shared.len(), 2);
        assert!(shared.contains_key("aliasIds"));
        assert!(shared.contains_key("elementId"));

        // Resolve shared properties
        resolve_shared_properties(&mut shapes, &shared);
        assert_eq!(shapes[0].properties.len(), 2);
    }

    #[test]
    fn test_strip_html() {
        let html = "<p>This is <code>some</code> HTML content.</p>";
        let stripped = strip_html(html);
        assert_eq!(stripped, "This is some HTML content.");
    }

    #[test]
    fn test_cardinality_parsing() {
        let ttl = r#"
:CardShape a oslc:ResourceShape ;
    oslc:describes oslc_kerml:Card ;
    oslc:property [ a oslc:Property ;
            oslc:name "zeroOrMany" ;
            oslc:occurs oslc:Zero-or-many ],
        [ a oslc:Property ;
            oslc:name "zeroOrOne" ;
            oslc:occurs oslc:Zero-or-one ],
        [ a oslc:Property ;
            oslc:name "exactlyOne" ;
            oslc:occurs oslc:Exactly-one ],
        [ a oslc:Property ;
            oslc:name "oneOrMany" ;
            oslc:occurs oslc:One-or-many ] .
"#;

        let (shapes, _) = parse_oslc_shapes(ttl).unwrap();
        assert_eq!(shapes[0].properties.len(), 4);

        let by_name: std::collections::HashMap<_, _> = shapes[0]
            .properties
            .iter()
            .map(|p| (p.name.as_str(), p))
            .collect();

        assert_eq!(by_name["zeroOrMany"].cardinality, Cardinality::ZeroOrMany);
        assert_eq!(by_name["zeroOrOne"].cardinality, Cardinality::ZeroOrOne);
        assert_eq!(by_name["exactlyOne"].cardinality, Cardinality::ExactlyOne);
        assert_eq!(by_name["oneOrMany"].cardinality, Cardinality::OneOrMany);
    }

    #[test]
    fn test_extract_local_name() {
        assert_eq!(extract_local_name(":aliasIds"), "aliasIds");
        assert_eq!(extract_local_name("oslc_kerml:Element"), "Element");
        assert_eq!(extract_local_name("Element"), "Element");
    }
}
