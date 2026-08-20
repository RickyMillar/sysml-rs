//! Specification coverage validation for SysML v2 / KerML.
//!
//! This module validates that our codegen pipeline has complete coverage of the
//! SysML v2 / KerML specifications by cross-checking multiple authoritative sources:
//! - TTL vocabulary files (Kerml-Vocab.ttl, SysML-vocab.ttl)
//! - XMI metamodel files (KerML.xmi, SysML.xmi)
//! - JSON Schema files (*.json in metamodel directory)
//!
//! ## Validation Checks
//!
//! 1. **Type Coverage**: All types in XMI must be in TTL and vice versa
//! 2. **Enum Coverage**: All enum values in TTL must match JSON schema and vice versa
//!
//! ## Usage
//!
//! ```ignore
//! use sysml_codegen::spec_validation::{validate_type_coverage, validate_enum_coverage};
//!
//! let type_report = validate_type_coverage(&ttl_types, &xmi_classes);
//! if !type_report.is_valid() {
//!     panic!("Type coverage errors: {}", type_report);
//! }
//! ```

use std::collections::HashSet;
use std::fmt;

/// Report on type coverage between TTL vocabulary and XMI metamodel.
#[derive(Debug, Clone)]
pub struct TypeCoverageReport {
    /// Types found in TTL but not in XMI
    pub ttl_only: Vec<String>,
    /// Types found in XMI but not in TTL
    pub xmi_only: Vec<String>,
    /// Number of types found in both sources
    pub matched_count: usize,
    /// Total types in TTL
    pub ttl_total: usize,
    /// Total types in XMI
    pub xmi_total: usize,
}

impl TypeCoverageReport {
    /// Returns true if all types match between TTL and XMI.
    pub fn is_valid(&self) -> bool {
        self.ttl_only.is_empty() && self.xmi_only.is_empty()
    }

    /// Returns true if there are any errors (types missing from either source).
    pub fn has_errors(&self) -> bool {
        !self.is_valid()
    }

    /// Returns the number of coverage errors.
    pub fn error_count(&self) -> usize {
        self.ttl_only.len() + self.xmi_only.len()
    }
}

impl fmt::Display for TypeCoverageReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Type Coverage Report:")?;
        writeln!(f, "  TTL types: {}", self.ttl_total)?;
        writeln!(f, "  XMI types: {}", self.xmi_total)?;
        writeln!(f, "  Matched:   {}", self.matched_count)?;

        if !self.ttl_only.is_empty() {
            writeln!(f, "  TTL-only ({}):", self.ttl_only.len())?;
            for t in &self.ttl_only {
                writeln!(f, "    - {}", t)?;
            }
        }

        if !self.xmi_only.is_empty() {
            writeln!(f, "  XMI-only ({}):", self.xmi_only.len())?;
            for t in &self.xmi_only {
                writeln!(f, "    - {}", t)?;
            }
        }

        if self.is_valid() {
            writeln!(f, "  Status: PASS")?;
        } else {
            writeln!(f, "  Status: FAIL ({} errors)", self.error_count())?;
        }

        Ok(())
    }
}

/// Report on enum coverage between TTL vocabulary and JSON schema.
#[derive(Debug, Clone)]
pub struct EnumCoverageReport {
    /// Individual enum validation results
    pub enums: Vec<EnumValidation>,
    /// Enums found in TTL but not in JSON
    pub ttl_only_enums: Vec<String>,
    /// Enums found in JSON but not in TTL
    pub json_only_enums: Vec<String>,
}

/// Validation result for a single enum type.
#[derive(Debug, Clone)]
pub struct EnumValidation {
    /// The name of the enum (e.g., "FeatureDirectionKind")
    pub name: String,
    /// Values found in TTL but not in JSON
    pub ttl_only: Vec<String>,
    /// Values found in JSON but not in TTL
    pub json_only: Vec<String>,
    /// Number of values found in both sources
    pub matched_count: usize,
    /// Total values in TTL
    pub ttl_total: usize,
    /// Total values in JSON
    pub json_total: usize,
}

impl EnumValidation {
    /// Returns true if all values match.
    pub fn is_valid(&self) -> bool {
        self.ttl_only.is_empty() && self.json_only.is_empty()
    }
}

impl EnumCoverageReport {
    /// Returns true if all enums and their values match.
    pub fn is_valid(&self) -> bool {
        self.ttl_only_enums.is_empty()
            && self.json_only_enums.is_empty()
            && self.enums.iter().all(|e| e.is_valid())
    }

    /// Returns true if there are any errors.
    pub fn has_errors(&self) -> bool {
        !self.is_valid()
    }

    /// Returns the number of coverage errors.
    pub fn error_count(&self) -> usize {
        let enum_errors = self.ttl_only_enums.len() + self.json_only_enums.len();
        let value_errors: usize = self
            .enums
            .iter()
            .map(|e| e.ttl_only.len() + e.json_only.len())
            .sum();
        enum_errors + value_errors
    }
}

impl fmt::Display for EnumCoverageReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Enum Coverage Report:")?;

        if !self.ttl_only_enums.is_empty() {
            writeln!(f, "  TTL-only enums ({}):", self.ttl_only_enums.len())?;
            for e in &self.ttl_only_enums {
                writeln!(f, "    - {}", e)?;
            }
        }

        if !self.json_only_enums.is_empty() {
            writeln!(f, "  JSON-only enums ({}):", self.json_only_enums.len())?;
            for e in &self.json_only_enums {
                writeln!(f, "    - {}", e)?;
            }
        }

        for enum_val in &self.enums {
            if !enum_val.is_valid() {
                writeln!(f, "  {}:", enum_val.name)?;
                if !enum_val.ttl_only.is_empty() {
                    writeln!(f, "    TTL-only values: {:?}", enum_val.ttl_only)?;
                }
                if !enum_val.json_only.is_empty() {
                    writeln!(f, "    JSON-only values: {:?}", enum_val.json_only)?;
                }
            }
        }

        if self.is_valid() {
            writeln!(f, "  Status: PASS")?;
        } else {
            writeln!(f, "  Status: FAIL ({} errors)", self.error_count())?;
        }

        Ok(())
    }
}

/// Validate type coverage between TTL vocabulary types and XMI classes.
///
/// Compares the set of type names from TTL with the set of class names from XMI.
/// Both sources should contain the same set of types.
///
/// # Arguments
///
/// * `ttl_type_names` - Type names extracted from TTL vocabulary files
/// * `xmi_class_names` - Class names extracted from XMI metamodel files
///
/// # Returns
///
/// A report detailing any mismatches between the two sources.
pub fn validate_type_coverage(
    ttl_type_names: &[String],
    xmi_class_names: &HashSet<String>,
) -> TypeCoverageReport {
    let ttl_set: HashSet<&String> = ttl_type_names.iter().collect();
    let xmi_set: HashSet<&String> = xmi_class_names.iter().collect();

    let ttl_only: Vec<String> = ttl_set.difference(&xmi_set).map(|s| (*s).clone()).collect();

    let xmi_only: Vec<String> = xmi_set.difference(&ttl_set).map(|s| (*s).clone()).collect();

    let matched_count = ttl_set.intersection(&xmi_set).count();

    let mut report = TypeCoverageReport {
        ttl_only,
        xmi_only,
        matched_count,
        ttl_total: ttl_type_names.len(),
        xmi_total: xmi_class_names.len(),
    };

    // Sort for consistent output
    report.ttl_only.sort();
    report.xmi_only.sort();

    report
}

/// Validate enum coverage between TTL vocabulary enums and JSON schema enums.
///
/// Compares enum names and their values from TTL with JSON schema.
/// Both sources should contain the same enums with the same values.
///
/// # Arguments
///
/// * `ttl_enums` - Enums extracted from TTL vocabulary files (name -> values)
/// * `json_enums` - Enums extracted from JSON schema files (name -> values)
///
/// # Returns
///
/// A report detailing any mismatches between the two sources.
#[allow(clippy::indexing_slicing)] // Bounds: keys come from intersection, guaranteed to exist in both maps
pub fn validate_enum_coverage(
    ttl_enums: &[(String, Vec<String>)],
    json_enums: &[(String, Vec<String>)],
) -> EnumCoverageReport {
    let ttl_enum_names: HashSet<&String> = ttl_enums.iter().map(|(name, _)| name).collect();
    let json_enum_names: HashSet<&String> = json_enums.iter().map(|(name, _)| name).collect();

    let ttl_only_enums: Vec<String> = ttl_enum_names
        .difference(&json_enum_names)
        .map(|s| (*s).clone())
        .collect();

    let json_only_enums: Vec<String> = json_enum_names
        .difference(&ttl_enum_names)
        .map(|s| (*s).clone())
        .collect();

    // Build maps for value comparison
    let ttl_map: std::collections::HashMap<&String, &Vec<String>> =
        ttl_enums.iter().map(|(k, v)| (k, v)).collect();
    let json_map: std::collections::HashMap<&String, &Vec<String>> =
        json_enums.iter().map(|(k, v)| (k, v)).collect();

    // Compare values for enums present in both
    let mut enums = Vec::new();
    for enum_name in ttl_enum_names.intersection(&json_enum_names) {
        let ttl_values: HashSet<&String> = ttl_map[enum_name].iter().collect();
        let json_values: HashSet<&String> = json_map[enum_name].iter().collect();

        let ttl_only: Vec<String> = ttl_values
            .difference(&json_values)
            .map(|s| (*s).clone())
            .collect();

        let json_only: Vec<String> = json_values
            .difference(&ttl_values)
            .map(|s| (*s).clone())
            .collect();

        let matched_count = ttl_values.intersection(&json_values).count();

        enums.push(EnumValidation {
            name: (*enum_name).clone(),
            ttl_only,
            json_only,
            matched_count,
            ttl_total: ttl_map[enum_name].len(),
            json_total: json_map[enum_name].len(),
        });
    }

    // Sort for consistent output
    enums.sort_by(|a, b| a.name.cmp(&b.name));

    let mut report = EnumCoverageReport {
        enums,
        ttl_only_enums,
        json_only_enums,
    };

    report.ttl_only_enums.sort();
    report.json_only_enums.sort();

    report
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_type_coverage_all_match() {
        let ttl_types = vec!["Element".to_owned(), "Relationship".to_owned()];
        let xmi_classes: HashSet<String> =
            ["Element".to_owned(), "Relationship".to_owned()].into();

        let report = validate_type_coverage(&ttl_types, &xmi_classes);

        assert!(report.is_valid());
        assert_eq!(report.matched_count, 2);
        assert!(report.ttl_only.is_empty());
        assert!(report.xmi_only.is_empty());
    }

    #[test]
    fn test_type_coverage_ttl_only() {
        let ttl_types = vec![
            "Element".to_owned(),
            "Relationship".to_owned(),
            "TtlOnly".to_owned(),
        ];
        let xmi_classes: HashSet<String> =
            ["Element".to_owned(), "Relationship".to_owned()].into();

        let report = validate_type_coverage(&ttl_types, &xmi_classes);

        assert!(!report.is_valid());
        assert_eq!(report.ttl_only, vec!["TtlOnly"]);
        assert!(report.xmi_only.is_empty());
    }

    #[test]
    fn test_type_coverage_xmi_only() {
        let ttl_types = vec!["Element".to_owned()];
        let xmi_classes: HashSet<String> = ["Element".to_owned(), "XmiOnly".to_owned()].into();

        let report = validate_type_coverage(&ttl_types, &xmi_classes);

        assert!(!report.is_valid());
        assert!(report.ttl_only.is_empty());
        assert_eq!(report.xmi_only, vec!["XmiOnly"]);
    }

    #[test]
    fn test_enum_coverage_all_match() {
        let ttl_enums = vec![(
            "FeatureDirectionKind".to_owned(),
            vec!["in".to_owned(), "out".to_owned(), "inout".to_owned()],
        )];
        let json_enums = vec![(
            "FeatureDirectionKind".to_owned(),
            vec!["in".to_owned(), "inout".to_owned(), "out".to_owned()],
        )];

        let report = validate_enum_coverage(&ttl_enums, &json_enums);

        assert!(report.is_valid());
        assert_eq!(report.enums.len(), 1);
        assert!(report.enums[0].is_valid());
    }

    #[test]
    fn test_enum_coverage_missing_enum() {
        let ttl_enums = vec![
            ("FeatureDirectionKind".to_owned(), vec!["in".to_owned()]),
            ("TtlOnlyKind".to_owned(), vec!["value1".to_owned()]),
        ];
        let json_enums = vec![
            ("FeatureDirectionKind".to_owned(), vec!["in".to_owned()]),
            ("JsonOnlyKind".to_owned(), vec!["value2".to_owned()]),
        ];

        let report = validate_enum_coverage(&ttl_enums, &json_enums);

        assert!(!report.is_valid());
        assert_eq!(report.ttl_only_enums, vec!["TtlOnlyKind"]);
        assert_eq!(report.json_only_enums, vec!["JsonOnlyKind"]);
    }

    #[test]
    fn test_enum_coverage_missing_value() {
        let ttl_enums = vec![(
            "FeatureDirectionKind".to_owned(),
            vec!["in".to_owned(), "out".to_owned(), "ttlOnly".to_owned()],
        )];
        let json_enums = vec![(
            "FeatureDirectionKind".to_owned(),
            vec!["in".to_owned(), "out".to_owned(), "jsonOnly".to_owned()],
        )];

        let report = validate_enum_coverage(&ttl_enums, &json_enums);

        assert!(!report.is_valid());
        assert_eq!(report.enums[0].ttl_only, vec!["ttlOnly"]);
        assert_eq!(report.enums[0].json_only, vec!["jsonOnly"]);
    }

    #[test]
    fn test_display_report() {
        let ttl_types = vec!["Element".to_owned(), "TtlOnly".to_owned()];
        let xmi_classes: HashSet<String> = ["Element".to_owned(), "XmiOnly".to_owned()].into();
        let report = validate_type_coverage(&ttl_types, &xmi_classes);

        let display = format!("{}", report);
        assert!(display.contains("TTL-only (1)"));
        assert!(display.contains("XMI-only (1)"));
        assert!(display.contains("FAIL"));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod integration_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_real_file_parsing() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir.join("../../..");
        let refs_dir = if repo_root.join("references").join("sysmlv2").exists() {
            repo_root.join("references").join("sysmlv2")
        } else {
            repo_root.join("docs").join("spec")
        };

        // Parse TTL
        let kerml_content = std::fs::read_to_string(refs_dir.join("Kerml-Vocab.ttl"))
            .expect("Failed to read KerML vocab");
        let sysml_content = std::fs::read_to_string(refs_dir.join("SysML-vocab.ttl"))
            .expect("Failed to read SysML vocab");

        let kerml_types = crate::parse_ttl_vocab(&kerml_content).unwrap();
        let sysml_types = crate::parse_ttl_vocab(&sysml_content).unwrap();

        println!("KerML types: {}", kerml_types.len());
        println!("SysML types: {}", sysml_types.len());

        // Check for WhileLoopActionUsage
        let has_while = sysml_types.iter().any(|t| t.name == "WhileLoopActionUsage");
        println!("WhileLoopActionUsage in sysml_types: {}", has_while);

        // Count unique non-Kind types
        let ttl_names: Vec<String> = kerml_types
            .iter()
            .chain(sysml_types.iter())
            .filter(|t| !t.name.ends_with("Kind"))
            .map(|t| t.name.clone())
            .collect();
        println!(
            "Total TTL type names (non-Kind, with dups): {}",
            ttl_names.len()
        );

        let ttl_set: HashSet<&String> = ttl_names.iter().collect();
        println!("Unique TTL types (non-Kind): {}", ttl_set.len());

        // Parse XMI
        let kerml_xmi = refs_dir.join("KerML/20250201/KerML.xmi");
        let sysml_xmi = refs_dir.join("SysML/20250201/SysML.xmi");
        let xmi_classes = crate::parse_all_xmi_classes(&kerml_xmi, &sysml_xmi).unwrap();
        println!("XMI classes: {}", xmi_classes.len());

        // Run validation
        let report = validate_type_coverage(&ttl_names, &xmi_classes);
        println!(
            "Report: matched={}, ttl_only={}, xmi_only={}",
            report.matched_count,
            report.ttl_only.len(),
            report.xmi_only.len()
        );
        println!("TTL-only: {:?}", report.ttl_only);
        println!("XMI-only: {:?}", report.xmi_only);

        // Check specifically for WhileLoopActionUsage
        // Verify WhileLoopActionUsage is found in both
        assert!(
            ttl_set.iter().any(|n| *n == "WhileLoopActionUsage"),
            "WhileLoopActionUsage should be in TTL"
        );
        assert!(
            xmi_classes.contains("WhileLoopActionUsage"),
            "WhileLoopActionUsage should be in XMI"
        );

        // Verify complete coverage
        assert!(
            report.is_valid(),
            "Type coverage should be complete. Report: {}",
            report
        );
    }
}
