//! Xtext cross-reference parser for extracting cross-reference patterns.
//!
//! This module parses Xtext grammar files (`.xtext`) to extract cross-reference
//! patterns that define how names are resolved to element references.
//!
//! ## Cross-Reference Pattern
//!
//! Xtext uses the pattern `property = [Namespace::Type | ReferenceName]` to define
//! cross-references. For example:
//!
//! ```text
//! general = [SysML::Type | QualifiedName]
//! ```
//!
//! This means the `general` property references a `Type` element resolved via
//! a `QualifiedName`.
//!
//! ## Multi-Valued References
//!
//! Multi-valued references use `+=` instead of `=`:
//!
//! ```text
//! client += [SysML::Element | QualifiedName]
//! ```

use std::collections::HashSet;

/// Information about a cross-reference extracted from xtext.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CrossReference {
    /// The property name (e.g., "general", "type", "subsettedFeature").
    pub property: String,
    /// The target type (e.g., "Type", "Feature", "Element").
    pub target_type: String,
    /// The namespace (e.g., "SysML", "KerML").
    pub namespace: String,
    /// The reference name used for resolution (e.g., "QualifiedName").
    pub reference_name: String,
    /// The containing rule name (e.g., "Specialization", "FeatureTyping").
    pub containing_rule: String,
    /// Whether this is a multi-valued reference (uses +=).
    pub is_multi: bool,
    /// Source file (for diagnostics).
    pub source_file: String,
    /// Line number in source (for diagnostics).
    pub line_number: usize,
}

/// Parse cross-references from Xtext grammar content.
///
/// # Arguments
///
/// * `content` - The xtext file content
/// * `source_file` - Name of the source file (for diagnostics)
///
/// # Returns
///
/// A vector of `CrossReference` structs.
pub fn parse_xtext_cross_references(content: &str, source_file: &str) -> Vec<CrossReference> {
    let mut cross_refs = Vec::new();
    let lines: Vec<&str> = content.lines().collect();

    let mut current_rule: Option<String> = None;

    for (line_idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // Skip comments and empty lines
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with("/*")
            || trimmed.starts_with("*")
        {
            continue;
        }

        // Detect rule definition start
        // Pattern: RuleName returns Namespace::Type : or RuleName :
        if let Some(rule_name) = parse_rule_start(trimmed) {
            current_rule = Some(rule_name);
            continue;
        }

        // Detect rule end
        if trimmed == ";" {
            current_rule = None;
            continue;
        }

        // Parse cross-references within the current rule
        if let Some(ref rule_name) = current_rule {
            for cross_ref in
                extract_cross_references_from_line(trimmed, rule_name, source_file, line_idx + 1)
            {
                cross_refs.push(cross_ref);
            }
        }
    }

    // Deduplicate by property name (keep first occurrence)
    deduplicate_cross_refs(cross_refs)
}

/// Parse a rule definition start and extract the rule name.
fn parse_rule_start(line: &str) -> Option<String> {
    // Skip terminal, fragment, enum rules
    if line.starts_with("terminal ") || line.starts_with("fragment ") || line.starts_with("enum ") {
        return None;
    }

    // Look for pattern: RuleName ... :
    // Find the colon that starts the rule body (not ::)
    let colon_pos = find_rule_colon(line)?;
    let before_colon = &line[..colon_pos];

    // Extract rule name (first identifier)
    let name = before_colon.split_whitespace().next()?;

    // Must start with uppercase (grammar rules)
    if name.chars().next()?.is_uppercase() && name.chars().all(|c| c.is_alphanumeric() || c == '_')
    {
        return Some(name.to_owned());
    }

    None
}

/// Find the position of the rule definition colon (not part of ::).
fn find_rule_colon(s: &str) -> Option<usize> {
    let chars: Vec<char> = s.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if ch == ':' {
            // Check if this is part of :: (namespace separator)
            let is_double_colon =
                (i > 0 && chars.get(i - 1) == Some(&':')) || chars.get(i + 1) == Some(&':');
            if !is_double_colon {
                return Some(i);
            }
        }
    }
    None
}

/// Extract cross-references from a single line.
fn extract_cross_references_from_line(
    line: &str,
    rule_name: &str,
    source_file: &str,
    line_number: usize,
) -> Vec<CrossReference> {
    let mut refs = Vec::new();

    // Find all [Namespace::Type | Name] patterns with their property names
    // Pattern: property (=|+=) [Namespace::Type | ReferenceName]
    let mut remaining = line;

    while let Some(bracket_start) = remaining.find('[') {
        let before_bracket = &remaining[..bracket_start];
        let after_bracket = &remaining[bracket_start..];

        // Find matching closing bracket
        if let Some(bracket_end) = after_bracket.find(']') {
            let bracket_content = &after_bracket[1..bracket_end];

            // Parse bracket content: Namespace::Type | ReferenceName
            if let Some((ns_type, ref_name)) = parse_bracket_content(bracket_content) {
                // Find property name and assignment operator
                if let Some((property, is_multi)) = find_property_assignment(before_bracket) {
                    let (namespace, target_type) = split_namespace_type(&ns_type);

                    refs.push(CrossReference {
                        property,
                        target_type,
                        namespace,
                        reference_name: ref_name,
                        containing_rule: rule_name.to_owned(),
                        is_multi,
                        source_file: source_file.to_owned(),
                        line_number,
                    });
                }
            }

            // Move past this bracket for the next iteration
            remaining = &remaining[bracket_start + bracket_end + 1..];
        } else {
            break;
        }
    }

    refs
}

/// Parse bracket content: "Namespace::Type | ReferenceName" or "Namespace::Type"
#[allow(clippy::indexing_slicing)] // Bounds: length checked before access
fn parse_bracket_content(content: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = content.split('|').collect();

    let ns_type = parts.first()?.trim().to_owned();
    let ref_name = if parts.len() > 1 {
        parts[1].trim().to_owned()
    } else {
        "ID".to_owned() // Default reference name
    };

    // Validate ns_type looks like Namespace::Type
    if ns_type.contains("::") {
        Some((ns_type, ref_name))
    } else {
        None
    }
}

/// Split "Namespace::Type" into (namespace, type).
#[allow(clippy::indexing_slicing)] // Bounds: length checked before access
fn split_namespace_type(ns_type: &str) -> (String, String) {
    let parts: Vec<&str> = ns_type.split("::").collect();
    if parts.len() >= 2 {
        (
            parts[0].trim().to_owned(),
            parts[1..].join("::").trim().to_owned(),
        )
    } else {
        ("".to_owned(), ns_type.to_owned())
    }
}

/// Find property name and whether it uses += (multi-valued).
fn find_property_assignment(before_bracket: &str) -> Option<(String, bool)> {
    let trimmed = before_bracket.trim();

    // Look for patterns: property += or property =
    // Working backwards from the bracket

    // Check for +=
    if let Some(pos) = trimmed.rfind("+=") {
        let property_part = trimmed[..pos].trim();
        let property = extract_last_identifier(property_part)?;
        return Some((property, true));
    }

    // Check for = (but not == or !=)
    if let Some(pos) = trimmed.rfind('=') {
        // Make sure it's not part of +=, ==, !=, etc.
        if pos > 0 {
            let prev_char = trimmed.chars().nth(pos - 1);
            if prev_char == Some('+') || prev_char == Some('=') || prev_char == Some('!') {
                return None;
            }
        }
        let property_part = trimmed[..pos].trim();
        let property = extract_last_identifier(property_part)?;
        return Some((property, false));
    }

    None
}

/// Extract the last valid identifier from a string.
fn extract_last_identifier(s: &str) -> Option<String> {
    // Find the last word that looks like an identifier
    s.split(|c: char| !c.is_alphanumeric() && c != '_')
        .rfind(|word| {
            !word.is_empty()
                && word
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphabetic() || c == '_')
        })
        .map(|s| s.to_owned())
}

/// Deduplicate cross-references, keeping the first occurrence of each property.
fn deduplicate_cross_refs(refs: Vec<CrossReference>) -> Vec<CrossReference> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut result = Vec::new();

    for cr in refs {
        if seen.insert(cr.property.clone()) {
            result.push(cr);
        }
    }

    result
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_crossref() {
        let content = r#"
Specialization returns SysML::Specialization :
    general = [SysML::Type | QualifiedName]
;
"#;
        let refs = parse_xtext_cross_references(content, "test.xtext");

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].property, "general");
        assert_eq!(refs[0].target_type, "Type");
        assert_eq!(refs[0].namespace, "SysML");
        assert_eq!(refs[0].reference_name, "QualifiedName");
        assert_eq!(refs[0].containing_rule, "Specialization");
        assert!(!refs[0].is_multi);
    }

    #[test]
    fn test_parse_multi_valued_crossref() {
        let content = r#"
Dependency returns SysML::Dependency :
    client += [SysML::Element|QualifiedName] ( ',' client += [SysML::Element|QualifiedName] )*
;
"#;
        let refs = parse_xtext_cross_references(content, "test.xtext");

        // Should find "client" (deduplicated)
        let client_ref = refs.iter().find(|r| r.property == "client");
        assert!(client_ref.is_some());
        let cr = client_ref.unwrap();
        assert!(cr.is_multi);
        assert_eq!(cr.target_type, "Element");
    }

    #[test]
    fn test_parse_multiple_crossrefs_same_rule() {
        let content = r#"
FeatureChainExpression returns SysML::FeatureChainExpression :
    referencedFeature = [SysML::Feature | QualifiedName] '.'
    crossedFeature = [SysML::Feature | QualifiedName]
;
"#;
        let refs = parse_xtext_cross_references(content, "test.xtext");

        assert_eq!(refs.len(), 2);

        let properties: Vec<_> = refs.iter().map(|r| r.property.as_str()).collect();
        assert!(properties.contains(&"referencedFeature"));
        assert!(properties.contains(&"crossedFeature"));
    }

    #[test]
    fn test_parse_crossref_without_space() {
        let content = r#"
Import returns SysML::Import :
    importedMembership = [SysML::Membership|QualifiedName]
;
"#;
        let refs = parse_xtext_cross_references(content, "test.xtext");

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].property, "importedMembership");
        assert_eq!(refs[0].target_type, "Membership");
    }

    #[test]
    fn test_parse_conjugated_port_definition() {
        let content = r#"
ConjugatedPortTyping returns SysML::ConjugatedPortTyping :
    conjugatedPortDefinition = [SysML::ConjugatedPortDefinition | ConjugatedQualifiedName]
;
"#;
        let refs = parse_xtext_cross_references(content, "test.xtext");

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].property, "conjugatedPortDefinition");
        assert_eq!(refs[0].target_type, "ConjugatedPortDefinition");
        assert_eq!(refs[0].reference_name, "ConjugatedQualifiedName");
    }

    #[test]
    fn test_find_property_assignment() {
        assert_eq!(
            find_property_assignment("  general = "),
            Some(("general".to_owned(), false))
        );
        assert_eq!(
            find_property_assignment("  client += "),
            Some(("client".to_owned(), true))
        );
        assert_eq!(
            find_property_assignment("( ',' client += "),
            Some(("client".to_owned(), true))
        );
    }

    #[test]
    fn test_parse_bracket_content() {
        assert_eq!(
            parse_bracket_content("SysML::Type | QualifiedName"),
            Some(("SysML::Type".to_owned(), "QualifiedName".to_owned()))
        );
        assert_eq!(
            parse_bracket_content("SysML::Element|QualifiedName"),
            Some(("SysML::Element".to_owned(), "QualifiedName".to_owned()))
        );
    }

    #[test]
    fn test_deduplication() {
        let content = r#"
Rule1 :
    prop = [SysML::Type | QualifiedName]
;
Rule2 :
    prop = [SysML::Type | QualifiedName]
;
"#;
        let refs = parse_xtext_cross_references(content, "test.xtext");

        // Should have deduplicated to just one "prop"
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].property, "prop");
    }
}
