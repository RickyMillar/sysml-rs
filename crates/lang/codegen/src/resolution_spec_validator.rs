//! Resolution spec completeness validation.
//!
//! This module validates that every `unresolved_*` property defined in the resolution
//! module has a corresponding entry in the cross-reference registry with an explicit
//! scoping strategy.
//!
//! ## Purpose
//!
//! The SysML v2 name resolution system uses `unresolved_*` properties to defer
//! reference resolution until the full model is available. This validator ensures:
//!
//! 1. Every unresolved property has a registry entry (for scoping strategy)
//! 2. No registry entries are missing (completeness check)
//! 3. Phase A vs Phase B properties are tracked separately
//!
//! ## Usage in build.rs
//!
//! ```ignore
//! use sysml_codegen::resolution_spec_validator::{
//!     extract_resolution_unresolved_props,
//!     validate_resolution_spec_completeness,
//!     ResolutionSpecValidationResult,
//! };
//!
//! let resolution_mod_content = fs::read_to_string("src/resolution/mod.rs")?;
//! let resolution_props = extract_resolution_unresolved_props(&resolution_mod_content);
//! let registry_props: Vec<String> = all_cross_refs.iter().map(|cr| cr.property.clone()).collect();
//!
//! let result = validate_resolution_spec_completeness(&resolution_props, &registry_props);
//!
//! if !result.is_valid() {
//!     panic!("Resolution spec validation failed: {}", result.format_report());
//! }
//! ```

use regex::Regex;
use std::collections::HashSet;

/// Property extracted from the resolution module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionProp {
    /// The constant name (e.g., "GENERAL", "TYPE").
    pub const_name: String,
    /// The unresolved property value (e.g., "unresolved_general").
    pub unresolved_value: String,
    /// The resolved property name (e.g., "general").
    pub resolved_name: String,
    /// Whether this is a Phase A property (parser-created).
    pub is_phase_a: bool,
}

/// Result of resolution spec validation.
#[derive(Debug, Clone)]
pub struct ResolutionSpecValidationResult {
    /// Properties in resolution module with registry entries.
    pub validated: Vec<String>,
    /// Properties in resolution module WITHOUT registry entries (critical).
    pub missing_from_registry: Vec<String>,
    /// Properties in registry WITHOUT resolution module definitions (warning).
    pub extra_in_registry: Vec<String>,
    /// Parser-created properties (Phase A, actively used).
    pub phase_a_props: Vec<String>,
    /// Resolution-defined properties not yet created by parser (Phase B).
    pub phase_b_props: Vec<String>,
    /// Total properties in resolution module.
    pub resolution_total: usize,
    /// Total properties in registry.
    pub registry_total: usize,
}

impl ResolutionSpecValidationResult {
    /// Check if validation passed (no missing registry entries).
    pub fn is_valid(&self) -> bool {
        self.missing_from_registry.is_empty()
    }

    /// Get coverage percentage.
    pub fn coverage_percent(&self) -> f64 {
        if self.resolution_total == 0 {
            100.0
        } else {
            (self.validated.len() as f64 / self.resolution_total as f64) * 100.0
        }
    }

    /// Format as a report string.
    pub fn format_report(&self) -> String {
        let mut report = String::new();

        report.push_str(&format!(
            "Resolution Spec Completeness: {}/{} properties have registry entries ({:.1}%)\n\n",
            self.validated.len(),
            self.resolution_total,
            self.coverage_percent()
        ));

        report.push_str(&format!(
            "Phase A (parser-created): {}\n",
            self.phase_a_props.len()
        ));
        for prop in &self.phase_a_props {
            report.push_str(&format!("  - {}\n", prop));
        }

        report.push_str(&format!(
            "\nPhase B (resolution-only): {}\n",
            self.phase_b_props.len()
        ));
        for prop in &self.phase_b_props {
            report.push_str(&format!("  - {}\n", prop));
        }

        if !self.missing_from_registry.is_empty() {
            report.push_str(&format!(
                "\n[ERROR] Missing from registry ({}): {:?}\n",
                self.missing_from_registry.len(),
                self.missing_from_registry
            ));
            report.push_str(
                "  These properties need corresponding entries in the cross-reference registry.\n",
            );
        }

        if !self.extra_in_registry.is_empty() {
            report.push_str(&format!(
                "\n[WARNING] Extra in registry ({}): {:?}\n",
                self.extra_in_registry.len(),
                self.extra_in_registry
            ));
            report.push_str("  These properties are in the registry but not defined as unresolved_* constants.\n");
            report.push_str(
                "  This may be intentional (owning-side properties, skipped properties).\n",
            );
        }

        report
    }

    /// Format a concise one-line summary.
    pub fn format_summary(&self) -> String {
        format!(
            "Resolution Spec: {}/{} validated ({:.1}%), {} Phase A, {} Phase B",
            self.validated.len(),
            self.resolution_total,
            self.coverage_percent(),
            self.phase_a_props.len(),
            self.phase_b_props.len(),
        )
    }
}

/// Extract `unresolved_*` property definitions from the resolution module source.
///
/// Parses lines like:
/// ```ignore
/// pub const GENERAL: &str = "unresolved_general";
/// ```
///
/// And groups them into Phase A (before "Phase B" comment) and Phase B (after).
///
/// # Arguments
///
/// * `source` - The source code of resolution/mod.rs
///
/// # Returns
///
/// A vector of `ResolutionProp` structs.
#[allow(clippy::expect_used)] // Invariant: regex literal is known-valid at compile time
pub fn extract_resolution_unresolved_props(source: &str) -> Vec<ResolutionProp> {
    let mut props = Vec::new();

    // Regex to match: pub const NAME: &str = "unresolved_*";
    let re = Regex::new(r#"pub const ([A-Z_]+): &str = "(unresolved_[a-zA-Z]+)";"#)
        .expect("Invalid regex");

    // Track if we've seen the Phase B marker
    let mut in_phase_b = false;
    let phase_b_marker = "Phase B";

    for line in source.lines() {
        // Check for Phase B marker in comments
        if line.contains(phase_b_marker) && line.trim().starts_with("//") {
            in_phase_b = true;
        }

        if let Some(caps) = re.captures(line) {
            let const_name = caps
                .get(1)
                .expect("invariant: regex group exists in match")
                .as_str()
                .to_owned();
            let unresolved_value = caps
                .get(2)
                .expect("invariant: regex group exists in match")
                .as_str()
                .to_owned();

            // Extract the resolved name by stripping "unresolved_" prefix
            let resolved_name = unresolved_value
                .strip_prefix("unresolved_")
                .unwrap_or(&unresolved_value)
                .to_owned();

            props.push(ResolutionProp {
                const_name,
                unresolved_value,
                resolved_name,
                is_phase_a: !in_phase_b,
            });
        }
    }

    props
}

/// Validate resolution spec completeness against the registry.
///
/// # Arguments
///
/// * `resolution_props` - Properties extracted from resolution module
/// * `registry_props` - Property names from the cross-reference registry
///
/// # Returns
///
/// A `ResolutionSpecValidationResult` with validation details.
pub fn validate_resolution_spec_completeness(
    resolution_props: &[ResolutionProp],
    registry_props: &[String],
) -> ResolutionSpecValidationResult {
    let resolution_names: HashSet<String> = resolution_props
        .iter()
        .map(|p| p.resolved_name.clone())
        .collect();

    let registry_set: HashSet<String> = registry_props.iter().cloned().collect();

    // Properties in both resolution module AND registry (validated)
    let validated: Vec<String> = resolution_names
        .intersection(&registry_set)
        .cloned()
        .collect();

    // Properties in resolution module but NOT in registry (critical)
    let missing_from_registry: Vec<String> = resolution_names
        .difference(&registry_set)
        .cloned()
        .collect();

    // Properties in registry but NOT in resolution module (warning)
    let extra_in_registry: Vec<String> = registry_set
        .difference(&resolution_names)
        .cloned()
        .collect();

    // Separate Phase A and Phase B properties
    let phase_a_props: Vec<String> = resolution_props
        .iter()
        .filter(|p| p.is_phase_a)
        .map(|p| p.resolved_name.clone())
        .collect();

    let phase_b_props: Vec<String> = resolution_props
        .iter()
        .filter(|p| !p.is_phase_a)
        .map(|p| p.resolved_name.clone())
        .collect();

    ResolutionSpecValidationResult {
        validated,
        missing_from_registry,
        extra_in_registry,
        phase_a_props,
        phase_b_props,
        resolution_total: resolution_props.len(),
        registry_total: registry_props.len(),
    }
}

/// Properties in the resolution module that are NOT cross-references.
///
/// These are special properties that don't need registry entries because:
/// - VALUE: FeatureValue contains an expression, not a cross-reference
/// - SOURCES/TARGETS: Base Relationship properties - not explicit grammar cross-refs.
///   Specific relationships use specialized properties (client/supplier, general/specific, etc.)
pub const NON_CROSSREF_RESOLUTION_PROPS: &[(&str, &str)] = &[
    (
        "value",
        "FeatureValue - contains expression, not a cross-reference",
    ),
    (
        "sources",
        "Base Relationship.source - covered by specialized props (client, general, etc.)",
    ),
    (
        "targets",
        "Base Relationship.target - covered by specialized props (supplier, specific, etc.)",
    ),
];

/// Resolution props that map to different registry names.
///
/// Some resolution props use different names than the registry.
pub const RESOLUTION_TO_REGISTRY_MAPPING: &[(&str, &str)] = &[
    // Currently empty - all properties either match or are excluded
];

/// Apply property name mappings for validation.
///
/// Some resolution props have different names in the registry.
/// Currently all properties either match directly or are excluded.
pub fn normalize_resolution_prop_name(name: &str) -> String {
    for (resolution_name, registry_name) in RESOLUTION_TO_REGISTRY_MAPPING {
        if name == *resolution_name {
            return registry_name.to_string();
        }
    }
    name.to_owned()
}

/// Check if a property is intentionally not a cross-reference.
pub fn is_non_crossref_prop(name: &str) -> bool {
    NON_CROSSREF_RESOLUTION_PROPS
        .iter()
        .any(|(n, _)| *n == name)
}

/// Validate with mappings and exclusions applied.
pub fn validate_resolution_spec_with_mappings(
    resolution_props: &[ResolutionProp],
    registry_props: &[String],
) -> ResolutionSpecValidationResult {
    // Filter out non-crossref props and apply mappings
    let filtered_props: Vec<ResolutionProp> = resolution_props
        .iter()
        .filter(|p| !is_non_crossref_prop(&p.resolved_name))
        .map(|p| ResolutionProp {
            const_name: p.const_name.clone(),
            unresolved_value: p.unresolved_value.clone(),
            resolved_name: normalize_resolution_prop_name(&p.resolved_name),
            is_phase_a: p.is_phase_a,
        })
        .collect();

    validate_resolution_spec_completeness(&filtered_props, registry_props)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_resolution_props() {
        let source = r#"
pub mod unresolved_props {
    // Phase A: parser-created
    pub const GENERAL: &str = "unresolved_general";
    pub const TYPE: &str = "unresolved_type";

    // Phase B: resolution-defined
    pub const SUPERCLASSIFIER: &str = "unresolved_superclassifier";
}
"#;

        let props = extract_resolution_unresolved_props(source);

        assert_eq!(props.len(), 3);

        let general = props.iter().find(|p| p.const_name == "GENERAL").unwrap();
        assert_eq!(general.resolved_name, "general");
        assert!(general.is_phase_a);

        let typ = props.iter().find(|p| p.const_name == "TYPE").unwrap();
        assert_eq!(typ.resolved_name, "type");
        assert!(typ.is_phase_a);

        let super_cls = props
            .iter()
            .find(|p| p.const_name == "SUPERCLASSIFIER")
            .unwrap();
        assert_eq!(super_cls.resolved_name, "superclassifier");
        assert!(!super_cls.is_phase_a);
    }

    #[test]
    fn test_validate_complete_coverage() {
        let props = vec![
            ResolutionProp {
                const_name: "GENERAL".to_owned(),
                unresolved_value: "unresolved_general".to_owned(),
                resolved_name: "general".to_owned(),
                is_phase_a: true,
            },
            ResolutionProp {
                const_name: "TYPE".to_owned(),
                unresolved_value: "unresolved_type".to_owned(),
                resolved_name: "type".to_owned(),
                is_phase_a: true,
            },
        ];

        let registry = vec!["general".to_owned(), "type".to_owned()];

        let result = validate_resolution_spec_completeness(&props, &registry);

        assert!(result.is_valid());
        assert_eq!(result.validated.len(), 2);
        assert_eq!(result.missing_from_registry.len(), 0);
        assert_eq!(result.coverage_percent(), 100.0);
    }

    #[test]
    fn test_validate_missing_from_registry() {
        let props = vec![
            ResolutionProp {
                const_name: "GENERAL".to_owned(),
                unresolved_value: "unresolved_general".to_owned(),
                resolved_name: "general".to_owned(),
                is_phase_a: true,
            },
            ResolutionProp {
                const_name: "MISSING".to_owned(),
                unresolved_value: "unresolved_missing".to_owned(),
                resolved_name: "missing".to_owned(),
                is_phase_a: true,
            },
        ];

        let registry = vec!["general".to_owned()];

        let result = validate_resolution_spec_completeness(&props, &registry);

        assert!(!result.is_valid());
        assert_eq!(result.validated.len(), 1);
        assert_eq!(result.missing_from_registry.len(), 1);
        assert!(result
            .missing_from_registry
            .contains(&"missing".to_owned()));
    }

    #[test]
    fn test_validate_extra_in_registry() {
        let props = vec![ResolutionProp {
            const_name: "GENERAL".to_owned(),
            unresolved_value: "unresolved_general".to_owned(),
            resolved_name: "general".to_owned(),
            is_phase_a: true,
        }];

        let registry = vec!["general".to_owned(), "extra".to_owned()];

        let result = validate_resolution_spec_completeness(&props, &registry);

        assert!(result.is_valid()); // Extra is just a warning
        assert_eq!(result.extra_in_registry.len(), 1);
        assert!(result.extra_in_registry.contains(&"extra".to_owned()));
    }

    #[test]
    fn test_phase_a_b_separation() {
        let props = vec![
            ResolutionProp {
                const_name: "GENERAL".to_owned(),
                unresolved_value: "unresolved_general".to_owned(),
                resolved_name: "general".to_owned(),
                is_phase_a: true,
            },
            ResolutionProp {
                const_name: "SUPERCLASSIFIER".to_owned(),
                unresolved_value: "unresolved_superclassifier".to_owned(),
                resolved_name: "superclassifier".to_owned(),
                is_phase_a: false,
            },
        ];

        let registry = vec!["general".to_owned(), "superclassifier".to_owned()];

        let result = validate_resolution_spec_completeness(&props, &registry);

        assert_eq!(result.phase_a_props.len(), 1);
        assert_eq!(result.phase_b_props.len(), 1);
        assert!(result.phase_a_props.contains(&"general".to_owned()));
        assert!(result
            .phase_b_props
            .contains(&"superclassifier".to_owned()));
    }

    #[test]
    fn test_normalize_prop_name() {
        // All properties pass through unchanged (mappings are now in exclusions)
        assert_eq!(normalize_resolution_prop_name("sources"), "sources");
        assert_eq!(normalize_resolution_prop_name("targets"), "targets");
        assert_eq!(normalize_resolution_prop_name("general"), "general");
    }

    #[test]
    fn test_is_non_crossref_prop() {
        assert!(is_non_crossref_prop("value"));
        assert!(is_non_crossref_prop("sources"));
        assert!(is_non_crossref_prop("targets"));
        assert!(!is_non_crossref_prop("general"));
        assert!(!is_non_crossref_prop("client"));
    }

    #[test]
    fn test_format_report() {
        let result = ResolutionSpecValidationResult {
            validated: vec!["general".to_owned()],
            missing_from_registry: vec!["missing".to_owned()],
            extra_in_registry: vec!["extra".to_owned()],
            phase_a_props: vec!["general".to_owned()],
            phase_b_props: vec![],
            resolution_total: 2,
            registry_total: 2,
        };

        let report = result.format_report();

        assert!(report.contains("Resolution Spec Completeness"));
        assert!(report.contains("[ERROR] Missing from registry"));
        assert!(report.contains("[WARNING] Extra in registry"));
    }
}
