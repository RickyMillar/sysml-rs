//! Cross-reference coverage validation.
//!
//! This module validates that all cross-references defined in the Xtext grammar
//! are handled by the name resolution implementation in sysml-core.
//!
//! ## Usage in build.rs
//!
//! ```ignore
//! use sysml_codegen::crossref_validation::{validate_crossref_coverage, CrossRefCoverageReport};
//!
//! let grammar_refs = parse_xtext_cross_references(&kerml_xtext, "KerML.xtext");
//! let impl_refs = ["general", "type", "subsettedFeature"]; // From resolution module
//!
//! let report = validate_crossref_coverage(&grammar_refs, &impl_refs);
//!
//! if !report.is_complete() {
//!     panic!("Missing cross-reference handlers: {:?}", report.unhandled);
//! }
//! ```

use crate::xtext_crossref_parser::CrossReference;
use std::collections::HashSet;

/// Coverage report for cross-reference validation.
#[derive(Debug, Clone)]
pub struct CrossRefCoverageReport {
    /// Properties that have resolution logic.
    pub handled: Vec<String>,
    /// Properties in grammar but missing resolution logic.
    pub unhandled: Vec<String>,
    /// Properties in implementation but not in grammar (possibly obsolete).
    pub extra: Vec<String>,
    /// Properties explicitly skipped (with reason).
    pub skipped: Vec<(String, String)>,
    /// Total grammar cross-references.
    pub grammar_total: usize,
    /// Total implementation handlers.
    pub impl_total: usize,
}

impl CrossRefCoverageReport {
    /// Check if coverage is complete (no unhandled properties).
    pub fn is_complete(&self) -> bool {
        self.unhandled.is_empty()
    }

    /// Get the number of unhandled properties.
    pub fn unhandled_count(&self) -> usize {
        self.unhandled.len()
    }

    /// Get the number of extra properties (not in grammar).
    pub fn extra_count(&self) -> usize {
        self.extra.len()
    }

    /// Get coverage percentage.
    pub fn coverage_percent(&self) -> f64 {
        if self.grammar_total == 0 {
            100.0
        } else {
            (self.handled.len() as f64 / self.grammar_total as f64) * 100.0
        }
    }

    /// Format as a summary string.
    pub fn summary(&self) -> String {
        format!(
            "Cross-reference coverage: {}/{} ({:.1}%)\n\
             Handled: {:?}\n\
             Unhandled: {:?}\n\
             Extra (not in grammar): {:?}\n\
             Skipped: {:?}",
            self.handled.len(),
            self.grammar_total,
            self.coverage_percent(),
            self.handled,
            self.unhandled,
            self.extra,
            self.skipped
        )
    }
}

/// Properties that are intentionally skipped with reasons.
///
/// These are cross-references that exist in the grammar but don't need
/// resolution in our implementation (e.g., they're handled differently,
/// or they're computed rather than stored).
pub const INTENTIONALLY_SKIPPED: &[(&str, &str)] = &[
    // Add properties here with reasons as implementation progresses
    // ("property", "reason"),
];

/// Validate cross-reference coverage.
///
/// # Arguments
///
/// * `grammar_refs` - Cross-references extracted from Xtext grammar
/// * `impl_refs` - Property names that have resolution logic
///
/// # Returns
///
/// A `CrossRefCoverageReport` with details about coverage.
pub fn validate_crossref_coverage(
    grammar_refs: &[CrossReference],
    impl_refs: &[&str],
) -> CrossRefCoverageReport {
    let grammar_properties: HashSet<String> =
        grammar_refs.iter().map(|r| r.property.clone()).collect();

    let impl_properties: HashSet<String> = impl_refs.iter().map(|s| s.to_string()).collect();

    let skipped_properties: HashSet<String> = INTENTIONALLY_SKIPPED
        .iter()
        .map(|(p, _)| p.to_string())
        .collect();

    // Properties that are in grammar AND implementation (handled)
    let handled: Vec<String> = grammar_properties
        .intersection(&impl_properties)
        .cloned()
        .collect();

    // Properties in grammar but NOT in implementation (and not skipped)
    let unhandled: Vec<String> = grammar_properties
        .difference(&impl_properties)
        .filter(|p| !skipped_properties.contains(*p))
        .cloned()
        .collect();

    // Properties in implementation but NOT in grammar
    let extra: Vec<String> = impl_properties
        .difference(&grammar_properties)
        .cloned()
        .collect();

    // Skipped properties with their reasons
    let skipped: Vec<(String, String)> = INTENTIONALLY_SKIPPED
        .iter()
        .filter(|(p, _)| grammar_properties.contains(*p))
        .map(|(p, r)| (p.to_string(), r.to_string()))
        .collect();

    CrossRefCoverageReport {
        handled,
        unhandled,
        extra,
        skipped,
        grammar_total: grammar_properties.len(),
        impl_total: impl_properties.len(),
    }
}

/// Group cross-references by their scoping strategy requirement.
///
/// This helps determine which scoping strategy each property needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScopeStrategy {
    /// Most common - resolve in owning namespace.
    OwningNamespace,
    /// Non-expression namespace (FeatureTyping, etc.).
    NonExpressionNamespace,
    /// Relative namespace for feature chains.
    RelativeNamespace,
    /// Feature chaining - position-dependent.
    FeatureChaining,
    /// Transition-specific scoping (SysML states).
    TransitionSpecific,
    /// Global/root package scope.
    Global,
}

impl ScopeStrategy {
    /// Get display name.
    pub fn as_str(&self) -> &'static str {
        match self {
            ScopeStrategy::OwningNamespace => "owning_namespace",
            ScopeStrategy::NonExpressionNamespace => "non_expression_namespace",
            ScopeStrategy::RelativeNamespace => "relative_namespace",
            ScopeStrategy::FeatureChaining => "feature_chaining",
            ScopeStrategy::TransitionSpecific => "transition_specific",
            ScopeStrategy::Global => "global",
        }
    }
}

/// Infer the likely scoping strategy for a cross-reference.
///
/// This uses heuristics based on property name and target type.
pub fn infer_scope_strategy(cr: &CrossReference) -> ScopeStrategy {
    let prop = cr.property.as_str();
    let _target = cr.target_type.as_str();
    let rule = cr.containing_rule.as_str();

    // Feature chaining rules
    if rule.contains("Chain") || prop == "crossedFeature" {
        return ScopeStrategy::FeatureChaining;
    }

    // Transition-specific
    if rule.contains("Transition")
        || prop.contains("trigger")
        || prop.contains("effect")
        || prop.contains("guard")
    {
        return ScopeStrategy::TransitionSpecific;
    }

    // FeatureTyping uses non-expression namespace
    if rule == "FeatureTyping" || prop == "type" {
        return ScopeStrategy::NonExpressionNamespace;
    }

    // Conjugation
    if prop.contains("conjugated") || prop.contains("original") {
        return ScopeStrategy::NonExpressionNamespace;
    }

    // Import-related
    if prop.contains("imported") {
        return ScopeStrategy::Global;
    }

    // Default: owning namespace
    ScopeStrategy::OwningNamespace
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn make_crossref(property: &str, target: &str, rule: &str) -> CrossReference {
        CrossReference {
            property: property.to_string(),
            target_type: target.to_string(),
            namespace: "SysML".to_owned(),
            reference_name: "QualifiedName".to_owned(),
            containing_rule: rule.to_string(),
            is_multi: false,
            source_file: "test.xtext".to_owned(),
            line_number: 1,
        }
    }

    #[test]
    fn test_validate_coverage_complete() {
        let grammar = vec![
            make_crossref("general", "Type", "Specialization"),
            make_crossref("type", "Type", "FeatureTyping"),
        ];

        let impl_refs = ["general", "type"];
        let report = validate_crossref_coverage(&grammar, &impl_refs);

        assert!(report.is_complete());
        assert_eq!(report.handled.len(), 2);
        assert_eq!(report.unhandled.len(), 0);
    }

    #[test]
    fn test_validate_coverage_missing() {
        let grammar = vec![
            make_crossref("general", "Type", "Specialization"),
            make_crossref("type", "Type", "FeatureTyping"),
            make_crossref("subsettedFeature", "Feature", "Subsetting"),
        ];

        let impl_refs = ["general"];
        let report = validate_crossref_coverage(&grammar, &impl_refs);

        assert!(!report.is_complete());
        assert_eq!(report.handled.len(), 1);
        assert_eq!(report.unhandled.len(), 2);
        assert!(report.unhandled.contains(&"type".to_owned()));
        assert!(report.unhandled.contains(&"subsettedFeature".to_owned()));
    }

    #[test]
    fn test_validate_coverage_extra() {
        let grammar = vec![make_crossref("general", "Type", "Specialization")];

        let impl_refs = ["general", "obsolete_property"];
        let report = validate_crossref_coverage(&grammar, &impl_refs);

        assert!(report.is_complete()); // Still complete (no unhandled grammar refs)
        assert_eq!(report.extra.len(), 1);
        assert!(report.extra.contains(&"obsolete_property".to_owned()));
    }

    #[test]
    fn test_infer_scope_strategy() {
        let cr1 = make_crossref("general", "Type", "Specialization");
        assert_eq!(infer_scope_strategy(&cr1), ScopeStrategy::OwningNamespace);

        let cr2 = make_crossref("type", "Type", "FeatureTyping");
        assert_eq!(
            infer_scope_strategy(&cr2),
            ScopeStrategy::NonExpressionNamespace
        );

        let cr3 = make_crossref("crossedFeature", "Feature", "FeatureChainExpression");
        assert_eq!(infer_scope_strategy(&cr3), ScopeStrategy::FeatureChaining);

        let cr4 = make_crossref("importedNamespace", "Namespace", "Import");
        assert_eq!(infer_scope_strategy(&cr4), ScopeStrategy::Global);
    }

    #[test]
    fn test_coverage_percent() {
        let grammar = vec![
            make_crossref("a", "Type", "R1"),
            make_crossref("b", "Type", "R2"),
            make_crossref("c", "Type", "R3"),
            make_crossref("d", "Type", "R4"),
        ];

        let impl_refs = ["a", "b"]; // 2 of 4
        let report = validate_crossref_coverage(&grammar, &impl_refs);

        assert!((report.coverage_percent() - 50.0).abs() < 0.1);
    }
}
