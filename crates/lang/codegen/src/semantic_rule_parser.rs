//! Parser for the semantic validation rules TOML catalog.
//!
//! Parses `semantic_rules.toml` into structured `SemanticRule` data
//! that the code generator uses to produce validation dispatchers.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// A semantic validation rule from the TOML catalog.
#[derive(Debug, Clone, Deserialize)]
pub struct SemanticRule {
    /// Unique rule identifier (e.g., "S001").
    pub id: String,
    /// The ElementKind this rule applies to (e.g., "Namespace", "Usage").
    pub element_type: String,
    /// Rule category (e.g., "distinguishability", "typing").
    pub category: String,
    /// Severity: "error" or "warning".
    pub severity: String,
    /// Human-readable error message template.
    /// May contain `{name}` placeholders for interpolation.
    pub message: String,
    /// Name of the check function to call (e.g., "unique_owned_member_names").
    pub check: String,
    /// Specification reference (e.g., "KerML 7.2.3").
    pub spec_ref: String,
}

/// Parsed rule catalog.
#[derive(Debug, Clone, Deserialize)]
struct RuleCatalog {
    rule: Vec<SemanticRule>,
}

/// Parse semantic rules from a TOML file.
pub fn parse_semantic_rules_file(path: &Path) -> Result<Vec<SemanticRule>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    parse_semantic_rules(&content)
}

/// Parse semantic rules from a TOML string.
pub fn parse_semantic_rules(content: &str) -> Result<Vec<SemanticRule>, String> {
    let catalog: RuleCatalog =
        toml::from_str(content).map_err(|e| format!("Failed to parse TOML: {}", e))?;
    Ok(catalog.rule)
}

/// Group rules by element type for dispatch generation.
pub fn group_rules_by_element_type(rules: &[SemanticRule]) -> HashMap<String, Vec<&SemanticRule>> {
    let mut map: HashMap<String, Vec<&SemanticRule>> = HashMap::new();
    for rule in rules {
        map.entry(rule.element_type.clone()).or_default().push(rule);
    }
    map
}

/// Get all unique check function names from the rules.
pub fn unique_check_functions(rules: &[SemanticRule]) -> Vec<String> {
    let mut checks: Vec<String> = rules.iter().map(|r| r.check.clone()).collect();
    checks.sort();
    checks.dedup();
    checks
}

/// Get all unique categories from the rules.
pub fn unique_categories(rules: &[SemanticRule]) -> Vec<String> {
    let mut cats: Vec<String> = rules.iter().map(|r| r.category.clone()).collect();
    cats.sort();
    cats.dedup();
    cats
}

/// Summary statistics for the rule catalog.
#[derive(Debug)]
pub struct RuleCatalogSummary {
    pub total_rules: usize,
    pub error_count: usize,
    pub warning_count: usize,
    pub categories: Vec<String>,
    pub element_types: Vec<String>,
    pub check_functions: Vec<String>,
}

/// Generate a summary of the rule catalog.
pub fn summarize_rules(rules: &[SemanticRule]) -> RuleCatalogSummary {
    let error_count = rules.iter().filter(|r| r.severity == "error").count();
    let warning_count = rules.iter().filter(|r| r.severity == "warning").count();

    let mut element_types: Vec<String> = rules.iter().map(|r| r.element_type.clone()).collect();
    element_types.sort();
    element_types.dedup();

    RuleCatalogSummary {
        total_rules: rules.len(),
        error_count,
        warning_count,
        categories: unique_categories(rules),
        element_types,
        check_functions: unique_check_functions(rules),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    const TEST_TOML: &str = r#"
[[rule]]
id = "S001"
element_type = "Namespace"
category = "distinguishability"
severity = "warning"
message = "Duplicate owned member name '{name}'"
check = "unique_owned_member_names"
spec_ref = "KerML 7.2.3"

[[rule]]
id = "S010"
element_type = "Usage"
category = "typing"
severity = "error"
message = "A usage must be typed by definitions"
check = "usage_typed_by_definitions"
spec_ref = "SysML 8.2.4"
"#;

    #[test]
    fn parse_rules_from_string() {
        let rules = parse_semantic_rules(TEST_TOML).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].id, "S001");
        assert_eq!(rules[0].element_type, "Namespace");
        assert_eq!(rules[0].severity, "warning");
        assert_eq!(rules[1].id, "S010");
        assert_eq!(rules[1].category, "typing");
    }

    #[test]
    fn group_by_element_type() {
        let rules = parse_semantic_rules(TEST_TOML).unwrap();
        let grouped = group_rules_by_element_type(&rules);
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped["Namespace"].len(), 1);
        assert_eq!(grouped["Usage"].len(), 1);
    }

    #[test]
    fn summarize() {
        let rules = parse_semantic_rules(TEST_TOML).unwrap();
        let summary = summarize_rules(&rules);
        assert_eq!(summary.total_rules, 2);
        assert_eq!(summary.error_count, 1);
        assert_eq!(summary.warning_count, 1);
        assert_eq!(summary.categories.len(), 2);
    }
}
