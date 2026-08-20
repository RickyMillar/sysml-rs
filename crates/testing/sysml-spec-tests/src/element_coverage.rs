//! Element kind coverage tracking.
//!
//! This module tracks which ElementKind variants are produced during parsing.
//! It derives the expected element kinds from the spec TTL vocabulary files
//! using the codegen crate.
//!
//! **Note**: This module requires the SysML v2 references directory to be available.
//! Set SYSML_CORPUS_PATH or ensure the references are in a standard location.

use std::collections::HashSet;
use std::path::Path;

use sysml_codegen::{parse_ttl_vocab, TypeInfo};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

/// Track which element kinds are produced during parsing.
pub struct ElementCoverageTracker {
    /// Set of element kinds that have been produced.
    produced_kinds: HashSet<String>,
}

impl ElementCoverageTracker {
    /// Create a new tracker.
    pub fn new() -> Self {
        ElementCoverageTracker {
            produced_kinds: HashSet::new(),
        }
    }

    /// Track element kinds from a parse result.
    pub fn track_parse(&mut self, source: &str) {
        let parser = TreeSitterParser::new();
        let file = SysmlFile::new("coverage.sysml", source);
        let result = parser.parse(&[file]);

        for (_, element) in result.graph.elements {
            self.produced_kinds.insert(format!("{:?}", element.kind));
        }
    }

    /// Track element kinds from multiple sources.
    pub fn track_all(&mut self, sources: &[&str]) {
        for source in sources {
            self.track_parse(source);
        }
    }

    /// Get all produced kinds.
    pub fn produced_kinds(&self) -> &HashSet<String> {
        &self.produced_kinds
    }

    /// Get the count of produced kinds.
    pub fn count(&self) -> usize {
        self.produced_kinds.len()
    }

    /// Check if a specific kind was produced.
    pub fn was_produced(&self, kind: &str) -> bool {
        self.produced_kinds.contains(kind)
    }

    /// Get kinds that were expected but not produced.
    pub fn missing_kinds(&self, expected: &HashSet<String>) -> HashSet<String> {
        expected.difference(&self.produced_kinds).cloned().collect()
    }
}

impl Default for ElementCoverageTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Path to KerML vocabulary file relative to the references root.
pub const KERML_VOCAB_PATH: &str = "Kerml-Vocab.ttl";

/// Path to SysML vocabulary file relative to the references root.
pub const SYSML_VOCAB_PATH: &str = "SysML-vocab.ttl";

/// Load element types from the TTL vocabulary files.
///
/// This uses the codegen crate's TTL parser to get the authoritative list
/// of element types from the SysML v2 specification.
///
/// # Arguments
///
/// * `references_path` - Path to the SysML v2 references directory
///
/// # Returns
///
/// A tuple of (kerml_types, sysml_types) with all type information.
pub fn load_element_types_from_spec(
    references_path: &Path,
) -> std::io::Result<(Vec<TypeInfo>, Vec<TypeInfo>)> {
    let kerml_path = references_path.join(KERML_VOCAB_PATH);
    let sysml_path = references_path.join(SYSML_VOCAB_PATH);

    let kerml_content = std::fs::read_to_string(kerml_path)?;
    let sysml_content = std::fs::read_to_string(sysml_path)?;

    let kerml_types = parse_ttl_vocab(&kerml_content)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    let sysml_types = parse_ttl_vocab(&sysml_content)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

    Ok((kerml_types, sysml_types))
}

/// Get all element kind names from the TTL vocabulary files.
///
/// This provides the authoritative list of all element types defined
/// in the SysML v2 specification.
///
/// # Arguments
///
/// * `references_path` - Path to the SysML v2 references directory
pub fn all_element_kinds_from_spec(references_path: &Path) -> std::io::Result<HashSet<String>> {
    let (kerml_types, sysml_types) = load_element_types_from_spec(references_path)?;

    let mut kinds = HashSet::new();
    for type_info in kerml_types.iter().chain(sysml_types.iter()) {
        kinds.insert(type_info.name.clone());
    }

    Ok(kinds)
}

/// Authoritative list of constructible element kinds.
/// Loaded from data/constructible_kinds.txt at compile time.
const CONSTRUCTIBLE_KINDS_DATA: &str = include_str!("../data/constructible_kinds.txt");

/// Get the list of all constructible element kinds.
///
/// These are the 77 kinds that have grammar rules mapping to them.
/// The list is loaded from data/constructible_kinds.txt (authoritative).
pub fn constructible_kinds() -> HashSet<String> {
    load_constructible_kinds_from_data(CONSTRUCTIBLE_KINDS_DATA)
}

/// Parse the constructible kinds from the data file content.
fn load_constructible_kinds_from_data(data: &str) -> HashSet<String> {
    data.lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|s| s.to_string())
        .collect()
}

/// Filter the full list of element kinds to those that are constructible
/// (i.e., have grammar rules that produce them).
///
/// Note: This function is kept for testing/validation purposes.
/// The authoritative list is in constructible_kinds.txt.
#[cfg(test)]
fn filter_to_constructible(all_kinds: &HashSet<String>) -> HashSet<String> {
    // The constructible kinds are a subset of all kinds - specifically those
    // that the parser can produce. For now, we filter to common ones.
    let constructible_prefixes = [
        "Package",
        "LibraryPackage",
        "Definition",
        "Usage",
        "Import",
        "Comment",
        "Documentation",
        "Dependency",
        "Metadata",
    ];

    let constructible_suffixes = ["Definition", "Usage"];

    all_kinds
        .iter()
        .filter(|kind| {
            constructible_prefixes.iter().any(|p| *kind == *p)
                || constructible_suffixes.iter().any(|s| kind.ends_with(s))
                || matches!(
                    kind.as_str(),
                    "Package"
                        | "LibraryPackage"
                        | "Import"
                        | "Comment"
                        | "Documentation"
                        | "Dependency"
                )
        })
        .cloned()
        .collect()
}

/// Categorize element kinds by their type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ElementCategory {
    /// Package types
    Package,
    /// Definition types (*Definition)
    Definition,
    /// Usage types (*Usage)
    Usage,
    /// Relationship types
    Relationship,
    /// Annotation types (Comment, Documentation)
    Annotation,
    /// Other types
    Other,
}

/// Categorize an element kind.
pub fn categorize_element(kind: &str) -> ElementCategory {
    if kind.contains("Package") {
        ElementCategory::Package
    } else if kind.ends_with("Definition") {
        ElementCategory::Definition
    } else if kind.ends_with("Usage") {
        ElementCategory::Usage
    } else if kind.contains("Relationship")
        || kind.contains("Membership")
        || kind.contains("Specialization")
        || kind.contains("Import")
    {
        ElementCategory::Relationship
    } else if kind == "Comment" || kind == "Documentation" || kind.contains("Annotation") {
        ElementCategory::Annotation
    } else {
        ElementCategory::Other
    }
}

/// Group element kinds by category.
pub fn elements_by_category(
    kinds: &HashSet<String>,
) -> std::collections::HashMap<ElementCategory, Vec<String>> {
    let mut grouped = std::collections::HashMap::new();

    for kind in kinds {
        let category = categorize_element(kind);
        grouped
            .entry(category)
            .or_insert_with(Vec::new)
            .push(kind.clone());
    }

    // Sort within each category
    for kinds in grouped.values_mut() {
        kinds.sort();
    }

    grouped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::try_find_references_dir;

    /// Skip test if spec files not available.
    fn require_spec_files() -> std::path::PathBuf {
        try_find_references_dir().expect(
            "Spec files required for this test. Set SYSML_CORPUS_PATH or \
             ensure references/sysmlv2 is available.",
        )
    }

    #[test]
    fn tracker_basic() {
        let mut tracker = ElementCoverageTracker::new();
        tracker.track_parse("package Test {}");
        assert!(tracker.was_produced("Package"));
    }

    #[test]
    fn constructible_kinds_non_empty() {
        let refs_dir = require_spec_files();
        let all_kinds = all_element_kinds_from_spec(&refs_dir).expect("Should load element kinds");
        let kinds = filter_to_constructible(&all_kinds);

        assert!(!kinds.is_empty());
        assert!(kinds.contains("Package"));
        assert!(kinds.contains("PartUsage"));
        assert!(kinds.contains("PartDefinition"));
    }

    #[test]
    fn all_element_kinds_loads_from_spec() {
        let refs_dir = require_spec_files();
        let (kerml, sysml) = load_element_types_from_spec(&refs_dir).expect("Should load types");

        // KerML has ~84 types, SysML has ~182 types
        // Note: sysml-core reports "84 KerML and 182 SysML types"
        println!(
            "Loaded {} KerML types, {} SysML types",
            kerml.len(),
            sysml.len()
        );
        assert!(
            kerml.len() >= 80,
            "Expected 80+ KerML types, got {}",
            kerml.len()
        );
        assert!(
            sysml.len() >= 180,
            "Expected 180+ SysML types, got {}",
            sysml.len()
        );

        // Combined set
        let kinds = all_element_kinds_from_spec(&refs_dir).expect("Should load element kinds");
        println!("Combined: {} unique types", kinds.len());

        // Check some known types
        assert!(kinds.contains("Element"), "Should have Element (KerML)");
        assert!(kinds.contains("Package"), "Should have Package");
        assert!(kinds.contains("PartUsage"), "Should have PartUsage (SysML)");
        assert!(
            kinds.contains("ActionDefinition"),
            "Should have ActionDefinition (SysML)"
        );
    }

    #[test]
    fn categorize_elements_correctly() {
        assert_eq!(categorize_element("Package"), ElementCategory::Package);
        assert_eq!(
            categorize_element("LibraryPackage"),
            ElementCategory::Package
        );
        assert_eq!(
            categorize_element("PartDefinition"),
            ElementCategory::Definition
        );
        assert_eq!(categorize_element("PartUsage"), ElementCategory::Usage);
        assert_eq!(categorize_element("Comment"), ElementCategory::Annotation);
        assert_eq!(
            categorize_element("Specialization"),
            ElementCategory::Relationship
        );
    }

    #[test]
    fn missing_kinds_detection() {
        let mut tracker = ElementCoverageTracker::new();
        tracker.track_parse("package Test {}");

        let expected: HashSet<String> = ["Package", "PartUsage", "ActionUsage"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        let missing = tracker.missing_kinds(&expected);
        assert!(!missing.contains("Package"));
        assert!(missing.contains("PartUsage"));
        assert!(missing.contains("ActionUsage"));
    }
}
