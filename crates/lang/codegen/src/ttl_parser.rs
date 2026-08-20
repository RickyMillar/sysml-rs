//! TTL (Turtle) vocabulary parser for SysML/KerML type definitions.
//!
//! This module parses TTL vocabulary files to extract type information
//! for code generation.

use std::collections::HashMap;

/// Information about a type extracted from TTL vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeInfo {
    /// The name of the type (e.g., "Element", "PartUsage").
    pub name: String,
    /// The supertypes this type extends.
    pub supertypes: Vec<String>,
    /// Optional comment/documentation for this type.
    pub comment: Option<String>,
}

/// Information about an enumeration type extracted from TTL vocabulary.
///
/// Enumerations in the SysML/KerML vocabulary are types whose name ends with "Kind"
/// (e.g., FeatureDirectionKind, VisibilityKind) and have specific values defined
/// as instances of that type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumInfo {
    /// The name of the enumeration type (e.g., "FeatureDirectionKind").
    pub name: String,
    /// The values/variants of this enumeration.
    pub values: Vec<EnumValue>,
    /// Optional comment/documentation for this enumeration.
    pub comment: Option<String>,
}

/// A single value/variant of an enumeration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumValue {
    /// The name of this enum value (e.g., "in", "out", "inout").
    pub name: String,
    /// Optional comment/documentation for this value.
    pub comment: Option<String>,
}

/// Error type for TTL parsing failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// Invalid TTL syntax.
    #[error("invalid TTL syntax: {0}")]
    InvalidSyntax(String),
    /// Missing required field.
    #[error("missing field: {0}")]
    MissingField(String),
}

/// Parse a TTL vocabulary file and extract type information.
///
/// # Arguments
///
/// * `content` - The TTL file content as a string
///
/// # Returns
///
/// A vector of `TypeInfo` structs representing the types defined in the vocabulary.
///
/// # Examples
///
/// ```
/// use sysml_codegen::ttl_parser::parse_ttl_vocab;
///
/// let ttl = r#"
/// @prefix oslc_kerml: <https://www.omg.org/spec/kerml/vocabulary#> .
/// @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
///
/// oslc_kerml:Element a rdfs:Class ;
///     rdfs:label "Element" ;
///     rdfs:subClassOf oslc_am:Resource .
/// "#;
///
/// let types = parse_ttl_vocab(ttl).unwrap();
/// assert_eq!(types.len(), 1);
/// assert_eq!(types[0].name, "Element");
/// ```
pub fn parse_ttl_vocab(content: &str) -> Result<Vec<TypeInfo>, ParseError> {
    let mut types = Vec::new();
    let mut current_type: Option<TypeInfo> = None;
    let mut in_multiline = false;
    let mut multiline_buffer = String::new();

    // Track prefixes for resolving type references
    let mut prefixes: HashMap<String, String> = HashMap::new();

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Handle prefix declarations
        if line.starts_with("@prefix") {
            if let Some((prefix, uri)) = parse_prefix_line(line) {
                prefixes.insert(prefix, uri);
            }
            continue;
        }

        // Handle multiline strings (comments with line breaks)
        if in_multiline {
            multiline_buffer.push_str(line);
            if line.contains("\"\"\"") {
                in_multiline = false;
                // Process the complete multiline string
                if let Some(ref mut t) = current_type {
                    if let Some(comment) = extract_multiline_comment(&multiline_buffer) {
                        t.comment = Some(comment);
                    }
                }
                multiline_buffer.clear();
            }
            continue;
        }

        if line.contains("\"\"\"") && !line.ends_with("\"\"\"") {
            in_multiline = true;
            multiline_buffer = line.to_owned();
            continue;
        }

        // Check if this is a new subject declaration (starts with oslc_ and contains " a ")
        // This includes both class declarations (a rdfs:Class) and other declarations (a rdf:Property, etc.)
        let is_new_subject = (line.starts_with("oslc_kerml:") || line.starts_with("oslc_sysml:"))
            && line.contains(" a ");

        // Check if this is specifically a new type declaration (contains "a rdfs:Class")
        let is_class_declaration = line.contains("a rdfs:Class");

        if is_new_subject {
            // Save the previous type if any (regardless of whether this is a class or property)
            if let Some(t) = current_type.take() {
                if !t.name.is_empty() {
                    types.push(t);
                }
            }

            // Only start tracking a new type if this is a class declaration
            if is_class_declaration {
                // Extract the type name from the line
                if let Some(name) = extract_type_name(line) {
                    current_type = Some(TypeInfo {
                        name,
                        supertypes: Vec::new(),
                        comment: None,
                    });
                }
            }
        }

        // Extract label (should match the type name)
        if line.contains("rdfs:label") {
            if let Some(ref mut t) = current_type {
                if let Some(label) = extract_quoted_string(line) {
                    // Use label as the canonical name
                    t.name = label;
                }
            }
        }

        // Extract supertypes (can be on same line or continuation lines)
        if line.contains("rdfs:subClassOf") {
            if let Some(ref mut t) = current_type {
                let supertypes = extract_supertypes(line);
                t.supertypes.extend(supertypes);
            }
        } else if current_type.is_some()
            && line.starts_with("oslc_")
            && !line.contains("a rdfs:Class")
            && !line.contains(" a ")
        // Don't treat new subject declarations as continuations
        {
            // This is a continuation line for supertypes (e.g., "oslc_kerml:Relationship .")
            if let Some(ref mut t) = current_type {
                // Extract type name from continuation
                if let Some(idx) = line.rfind(':') {
                    let name = line[idx + 1..]
                        .trim()
                        .trim_end_matches(';')
                        .trim_end_matches(',')
                        .trim_end_matches('.')
                        .trim();
                    if !name.is_empty() {
                        t.supertypes.push(name.to_owned());
                    }
                }
            }
        }

        // Extract comment (single line)
        if line.contains("rdfs:comment") && !in_multiline {
            if let Some(ref mut t) = current_type {
                if let Some(comment) = extract_quoted_string(line) {
                    t.comment = Some(comment);
                }
            }
        }

        // Check for end of type declaration (line ends with ".")
        if line.ends_with('.') && current_type.is_some() {
            // We might still need to capture the last supertype or comment
        }
    }

    // Don't forget the last type
    if let Some(t) = current_type {
        if !t.name.is_empty() && is_valid_rust_identifier(&t.name) {
            types.push(t);
        }
    }

    // Filter out invalid identifiers (e.g., names with spaces from ontology metadata)
    types.retain(|t| is_valid_rust_identifier(&t.name));

    // Sort by name for consistent output
    types.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(types)
}

/// Check if a string is a valid Rust identifier (used for enum variants).
#[allow(clippy::expect_used)] // Invariant: non-empty string after is_empty check
fn is_valid_rust_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    // Must start with a letter or underscore
    let mut chars = s.chars();
    let first = chars
        .next()
        .expect("invariant: non-empty string after length check");
    if !first.is_alphabetic() && first != '_' {
        return false;
    }

    // Rest must be alphanumeric or underscore, and no spaces
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Parse a @prefix line and extract the prefix and URI.
#[allow(clippy::indexing_slicing)] // Bounds: parts.len() == 2 checked before access
fn parse_prefix_line(line: &str) -> Option<(String, String)> {
    // Format: @prefix name: <uri> .
    let line = line.trim_start_matches("@prefix").trim();
    let parts: Vec<&str> = line.splitn(2, ':').collect();
    if parts.len() == 2 {
        let prefix = parts[0].trim().to_owned();
        let uri = parts[1]
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>')
            .trim_end_matches('.')
            .trim()
            .to_owned();
        Some((prefix, uri))
    } else {
        None
    }
}

/// Extract a type name from a line like "oslc_kerml:Element a rdfs:Class ;"
fn extract_type_name(line: &str) -> Option<String> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if let Some(first) = parts.first() {
        // Extract the local name after the prefix
        if let Some(idx) = first.rfind(':') {
            return Some(first[idx + 1..].to_string());
        }
    }
    None
}

/// Extract a quoted string from a line.
fn extract_quoted_string(line: &str) -> Option<String> {
    // Handle triple-quoted strings first
    if line.contains("\"\"\"") {
        let start = line.find("\"\"\"")? + 3;
        let rest = &line[start..];
        let end = rest.find("\"\"\"")?;
        return Some(rest[..end].to_string());
    }

    // Handle single-quoted strings
    let start = line.find('"')? + 1;
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Extract a multiline comment from a buffer.
fn extract_multiline_comment(buffer: &str) -> Option<String> {
    let start = buffer.find("\"\"\"")? + 3;
    let rest = &buffer[start..];
    let end = rest.find("\"\"\"")?;
    Some(rest[..end].trim().to_owned())
}

/// Extract supertypes from a rdfs:subClassOf line.
fn extract_supertypes(line: &str) -> Vec<String> {
    let mut supertypes = Vec::new();

    // Find the part after "rdfs:subClassOf"
    if let Some(idx) = line.find("rdfs:subClassOf") {
        let rest = &line[idx + "rdfs:subClassOf".len()..];

        // Split on commas for multiple supertypes
        for part in rest.split(',') {
            let part = part
                .trim()
                .trim_end_matches(';')
                .trim_end_matches('.')
                .trim();

            // Extract the local name
            if let Some(idx) = part.rfind(':') {
                let name = part[idx + 1..].trim();
                if !name.is_empty() {
                    supertypes.push(name.to_owned());
                }
            }
        }
    }

    supertypes
}

/// Parse a TTL vocabulary file and extract enumeration types and their values.
///
/// Enumerations are identified as types whose names end with "Kind" (e.g., FeatureDirectionKind).
/// Their values are instances declared using the pattern `prefix:value a prefix:EnumKind`.
///
/// # Arguments
///
/// * `content` - The TTL file content as a string
///
/// # Returns
///
/// A vector of `EnumInfo` structs representing the enumeration types and their values.
///
/// # Examples
///
/// ```
/// use sysml_codegen::ttl_parser::parse_ttl_enums;
///
/// let ttl = r#"
/// @prefix oslc_kerml: <https://www.omg.org/spec/kerml/vocabulary#> .
/// @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
///
/// oslc_kerml:FeatureDirectionKind a rdfs:Class ;
///     rdfs:label "FeatureDirectionKind" ;
///     rdfs:comment "Enumerates direction kinds." .
///
/// oslc_kerml:in a oslc_kerml:FeatureDirectionKind ;
///     rdfs:label "in" ;
///     rdfs:comment "Input direction." .
///
/// oslc_kerml:out a oslc_kerml:FeatureDirectionKind ;
///     rdfs:label "out" ;
///     rdfs:comment "Output direction." .
/// "#;
///
/// let enums = parse_ttl_enums(ttl).unwrap();
/// assert_eq!(enums.len(), 1);
/// assert_eq!(enums[0].name, "FeatureDirectionKind");
/// assert_eq!(enums[0].values.len(), 2);
/// ```
pub fn parse_ttl_enums(content: &str) -> Result<Vec<EnumInfo>, ParseError> {
    // First pass: find all enum types (classes ending with "Kind")
    let mut enum_types: HashMap<String, EnumInfo> = HashMap::new();
    // Map from local name to (enum_kind_name, enum_value_info)
    let mut enum_values: HashMap<String, (String, EnumValue)> = HashMap::new();

    let mut current_item: Option<(String, Option<String>, Option<String>)> = None; // (name, label, comment)
    let mut current_is_enum_type = false;
    let mut current_enum_kind: Option<String> = None; // If this is an enum value, which enum kind

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Skip prefix declarations
        if line.starts_with("@prefix") {
            continue;
        }

        // Check for a new subject declaration (starts with prefix:name)
        if (line.starts_with("oslc_kerml:") || line.starts_with("oslc_sysml:"))
            && !line.starts_with("oslc_kerml: .")
            && !line.starts_with("oslc_sysml: .")
        {
            // Save the previous item if complete
            if let Some((name, label, comment)) = current_item.take() {
                let actual_name = label.unwrap_or(name.clone());

                if current_is_enum_type && actual_name.ends_with("Kind") {
                    // This is an enum type declaration
                    enum_types.entry(actual_name.clone()).or_insert(EnumInfo {
                        name: actual_name,
                        values: Vec::new(),
                        comment,
                    });
                } else if let Some(kind) = current_enum_kind.take() {
                    // This is an enum value
                    enum_values.insert(
                        actual_name.clone(),
                        (
                            kind,
                            EnumValue {
                                name: actual_name,
                                comment,
                            },
                        ),
                    );
                }
            }

            // Extract the local name
            let local_name = if let Some(idx) = line.find(':') {
                let rest = &line[idx + 1..];
                rest.split_whitespace().next().unwrap_or("").to_owned()
            } else {
                continue;
            };

            // Check if this is a class declaration (enum type) or an instance (enum value)
            current_is_enum_type = line.contains("a rdfs:Class");
            current_enum_kind = None;

            // Check if this is an instance of an enum kind
            // Pattern: "oslc_*:value a oslc_*:SomeKind ;"
            if !current_is_enum_type {
                // Look for "a oslc_*:*Kind"
                if let Some(a_idx) = line.find(" a ") {
                    let type_part = &line[a_idx + 3..];
                    // Extract the type name
                    if let Some(colon_idx) = type_part.find(':') {
                        let type_name = type_part[colon_idx + 1..]
                            .split_whitespace()
                            .next()
                            .unwrap_or("")
                            .trim_end_matches(';')
                            .trim_end_matches('.');
                        if type_name.ends_with("Kind") {
                            current_enum_kind = Some(type_name.to_owned());
                        }
                    }
                }
            }

            current_item = Some((local_name, None, None));
        }

        // Extract label
        if line.contains("rdfs:label") {
            if let Some((_, ref mut label, _)) = current_item {
                if let Some(l) = extract_quoted_string(line) {
                    *label = Some(l);
                }
            }
        }

        // Extract comment (single line)
        if line.contains("rdfs:comment") && !line.contains("\"\"\"") {
            if let Some((_, _, ref mut comment)) = current_item {
                if let Some(c) = extract_quoted_string(line) {
                    *comment = Some(c);
                }
            }
        }
    }

    // Don't forget the last item
    if let Some((name, label, comment)) = current_item {
        let actual_name = label.unwrap_or(name);

        if current_is_enum_type && actual_name.ends_with("Kind") {
            enum_types.entry(actual_name.clone()).or_insert(EnumInfo {
                name: actual_name,
                values: Vec::new(),
                comment,
            });
        } else if let Some(kind) = current_enum_kind {
            enum_values.insert(
                actual_name.clone(),
                (
                    kind,
                    EnumValue {
                        name: actual_name,
                        comment,
                    },
                ),
            );
        }
    }

    // Now associate values with their enum types
    for (_, (kind_name, value)) in enum_values {
        if let Some(enum_info) = enum_types.get_mut(&kind_name) {
            enum_info.values.push(value);
        }
    }

    // Sort values within each enum for consistent output
    for enum_info in enum_types.values_mut() {
        enum_info.values.sort_by(|a, b| a.name.cmp(&b.name));
    }

    // Convert to a sorted vector
    let mut result: Vec<EnumInfo> = enum_types.into_values().collect();
    result.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(result)
}

/// Merge enumeration info from two sources (e.g., KerML and SysML), deduplicating.
///
/// When the same enum type appears in both sources, the values are merged.
pub fn merge_enum_info(enums1: Vec<EnumInfo>, enums2: Vec<EnumInfo>) -> Vec<EnumInfo> {
    let mut merged: HashMap<String, EnumInfo> = HashMap::new();

    for enum_info in enums1 {
        merged.insert(enum_info.name.clone(), enum_info);
    }

    for enum_info in enums2 {
        if let Some(existing) = merged.get_mut(&enum_info.name) {
            // Merge values, avoiding duplicates
            for value in enum_info.values {
                if !existing.values.iter().any(|v| v.name == value.name) {
                    existing.values.push(value);
                }
            }
            // Update comment if the existing one is None
            if existing.comment.is_none() && enum_info.comment.is_some() {
                existing.comment = enum_info.comment;
            }
        } else {
            merged.insert(enum_info.name.clone(), enum_info);
        }
    }

    // Sort values within each enum
    for enum_info in merged.values_mut() {
        enum_info.values.sort_by(|a, b| a.name.cmp(&b.name));
    }

    let mut result: Vec<EnumInfo> = merged.into_values().collect();
    result.sort_by(|a, b| a.name.cmp(&b.name));
    result
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_type() {
        let ttl = r#"
@prefix oslc_kerml: <https://www.omg.org/spec/kerml/vocabulary#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

oslc_kerml:Element a rdfs:Class ;
    rdfs:label "Element" ;
    rdfs:comment "An Element is a constituent of a model." ;
    rdfs:subClassOf oslc_am:Resource .
"#;

        let types = parse_ttl_vocab(ttl).unwrap();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].name, "Element");
        assert_eq!(types[0].supertypes, vec!["Resource"]);
        assert!(types[0].comment.is_some());
    }

    #[test]
    fn parse_multiple_supertypes() {
        let ttl = r#"
oslc_kerml:Connector a rdfs:Class ;
    rdfs:label "Connector" ;
    rdfs:subClassOf oslc_kerml:Feature,
        oslc_kerml:Relationship .
"#;

        let types = parse_ttl_vocab(ttl).unwrap();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].name, "Connector");
        assert_eq!(types[0].supertypes.len(), 2);
        assert!(types[0].supertypes.contains(&"Feature".to_owned()));
        assert!(types[0].supertypes.contains(&"Relationship".to_owned()));
    }

    #[test]
    fn parse_multiple_types() {
        let ttl = r#"
oslc_kerml:Element a rdfs:Class ;
    rdfs:label "Element" ;
    rdfs:subClassOf oslc_am:Resource .

oslc_kerml:Relationship a rdfs:Class ;
    rdfs:label "Relationship" ;
    rdfs:subClassOf oslc_kerml:Element .

oslc_kerml:Feature a rdfs:Class ;
    rdfs:label "Feature" ;
    rdfs:subClassOf oslc_kerml:Type .
"#;

        let types = parse_ttl_vocab(ttl).unwrap();
        assert_eq!(types.len(), 3);

        // Types should be sorted alphabetically
        assert_eq!(types[0].name, "Element");
        assert_eq!(types[1].name, "Feature");
        assert_eq!(types[2].name, "Relationship");
    }

    #[test]
    fn extract_type_name_test() {
        assert_eq!(
            extract_type_name("oslc_kerml:Element a rdfs:Class ;"),
            Some("Element".to_owned())
        );
        assert_eq!(
            extract_type_name("oslc_sysml:PartUsage a rdfs:Class ;"),
            Some("PartUsage".to_owned())
        );
    }

    #[test]
    fn extract_quoted_string_test() {
        assert_eq!(
            extract_quoted_string("rdfs:label \"Element\" ;"),
            Some("Element".to_owned())
        );
        assert_eq!(
            extract_quoted_string("rdfs:comment \"A simple comment.\" ;"),
            Some("A simple comment.".to_owned())
        );
    }

    #[test]
    fn extract_supertypes_test() {
        assert_eq!(
            extract_supertypes("rdfs:subClassOf oslc_kerml:Type ."),
            vec!["Type"]
        );
        assert_eq!(
            extract_supertypes("rdfs:subClassOf oslc_kerml:Feature, oslc_kerml:Relationship ."),
            vec!["Feature", "Relationship"]
        );
    }

    #[test]
    fn is_valid_rust_identifier_test() {
        assert!(is_valid_rust_identifier("Element"));
        assert!(is_valid_rust_identifier("PartUsage"));
        assert!(is_valid_rust_identifier("_private"));
        assert!(is_valid_rust_identifier("Type123"));

        assert!(!is_valid_rust_identifier(""));
        assert!(!is_valid_rust_identifier("OSLC KerML Vocabulary")); // Has spaces
        assert!(!is_valid_rust_identifier("123Type")); // Starts with number
        assert!(!is_valid_rust_identifier("Type-Name")); // Has hyphen
    }

    #[test]
    fn filters_invalid_identifiers() {
        // Test that invalid identifiers like "OSLC KerML Vocabulary" are filtered out
        let ttl = r#"
oslc_kerml:Element a rdfs:Class ;
    rdfs:label "Element" .
"#;

        let types = parse_ttl_vocab(ttl).unwrap();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].name, "Element");
    }

    #[test]
    fn filters_names_with_spaces() {
        // If a label contains spaces (like from ontology metadata), it should be filtered
        let ttl = r#"
oslc_kerml:BadName a rdfs:Class ;
    rdfs:label "OSLC KerML Vocabulary" .

oslc_kerml:GoodName a rdfs:Class ;
    rdfs:label "GoodName" .
"#;

        let types = parse_ttl_vocab(ttl).unwrap();
        // Only GoodName should be kept (label takes precedence over local name)
        let names: Vec<&str> = types.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"GoodName"));
        assert!(!names.iter().any(|n| n.contains(' ')));
    }

    #[test]
    fn parse_enum_type_and_values() {
        let ttl = r#"
@prefix oslc_kerml: <https://www.omg.org/spec/kerml/vocabulary#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

oslc_kerml:FeatureDirectionKind a rdfs:Class ;
    rdfs:label "FeatureDirectionKind" ;
    rdfs:comment "Enumerates direction kinds." .

oslc_kerml:in a oslc_kerml:FeatureDirectionKind ;
    rdfs:label "in" ;
    rdfs:comment "Input direction." .

oslc_kerml:out a oslc_kerml:FeatureDirectionKind ;
    rdfs:label "out" ;
    rdfs:comment "Output direction." .

oslc_kerml:inout a oslc_kerml:FeatureDirectionKind ;
    rdfs:label "inout" ;
    rdfs:comment "Input/output direction." .
"#;

        let enums = parse_ttl_enums(ttl).unwrap();
        assert_eq!(enums.len(), 1);

        let feature_dir = &enums[0];
        assert_eq!(feature_dir.name, "FeatureDirectionKind");
        assert_eq!(feature_dir.values.len(), 3);
        assert_eq!(
            feature_dir.comment,
            Some("Enumerates direction kinds.".to_owned())
        );

        // Values should be sorted alphabetically
        let value_names: Vec<&str> = feature_dir.values.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(value_names, vec!["in", "inout", "out"]);
    }

    #[test]
    fn parse_multiple_enums() {
        let ttl = r#"
oslc_kerml:FeatureDirectionKind a rdfs:Class ;
    rdfs:label "FeatureDirectionKind" .

oslc_kerml:VisibilityKind a rdfs:Class ;
    rdfs:label "VisibilityKind" .

oslc_kerml:in a oslc_kerml:FeatureDirectionKind ;
    rdfs:label "in" .

oslc_kerml:out a oslc_kerml:FeatureDirectionKind ;
    rdfs:label "out" .

oslc_kerml:public a oslc_kerml:VisibilityKind ;
    rdfs:label "public" .

oslc_kerml:private a oslc_kerml:VisibilityKind ;
    rdfs:label "private" .
"#;

        let enums = parse_ttl_enums(ttl).unwrap();
        assert_eq!(enums.len(), 2);

        // Enums should be sorted by name
        assert_eq!(enums[0].name, "FeatureDirectionKind");
        assert_eq!(enums[1].name, "VisibilityKind");

        assert_eq!(enums[0].values.len(), 2);
        assert_eq!(enums[1].values.len(), 2);
    }

    #[test]
    fn merge_enum_info_test() {
        let kerml_enums = vec![EnumInfo {
            name: "FeatureDirectionKind".to_owned(),
            values: vec![
                EnumValue {
                    name: "in".to_owned(),
                    comment: None,
                },
                EnumValue {
                    name: "out".to_owned(),
                    comment: None,
                },
            ],
            comment: Some("KerML comment".to_owned()),
        }];

        let sysml_enums = vec![
            EnumInfo {
                name: "FeatureDirectionKind".to_owned(),
                values: vec![
                    EnumValue {
                        name: "in".to_owned(),
                        comment: None,
                    }, // duplicate
                    EnumValue {
                        name: "inout".to_owned(),
                        comment: None,
                    }, // new
                ],
                comment: None,
            },
            EnumInfo {
                name: "PortionKind".to_owned(),
                values: vec![EnumValue {
                    name: "snapshot".to_owned(),
                    comment: None,
                }],
                comment: Some("SysML only".to_owned()),
            },
        ];

        let merged = merge_enum_info(kerml_enums, sysml_enums);
        assert_eq!(merged.len(), 2);

        // FeatureDirectionKind should have 3 unique values
        let fdk = merged
            .iter()
            .find(|e| e.name == "FeatureDirectionKind")
            .unwrap();
        assert_eq!(fdk.values.len(), 3);
        assert_eq!(fdk.comment, Some("KerML comment".to_owned())); // Kept from first

        // PortionKind should be included
        let pk = merged.iter().find(|e| e.name == "PortionKind").unwrap();
        assert_eq!(pk.values.len(), 1);
    }

    #[test]
    fn ignores_non_kind_classes() {
        let ttl = r#"
oslc_kerml:Element a rdfs:Class ;
    rdfs:label "Element" .

oslc_kerml:FeatureDirectionKind a rdfs:Class ;
    rdfs:label "FeatureDirectionKind" .

oslc_kerml:in a oslc_kerml:FeatureDirectionKind ;
    rdfs:label "in" .
"#;

        let enums = parse_ttl_enums(ttl).unwrap();
        // Should only find FeatureDirectionKind, not Element
        assert_eq!(enums.len(), 1);
        assert_eq!(enums[0].name, "FeatureDirectionKind");
    }

    #[test]
    fn is_valid_rust_identifier_edge_cases() {
        assert!(!is_valid_rust_identifier(""));
        assert!(is_valid_rust_identifier("a"));
        assert!(is_valid_rust_identifier("_"));
        assert!(!is_valid_rust_identifier("1abc"));
        assert!(!is_valid_rust_identifier("foo bar"));
    }
}
