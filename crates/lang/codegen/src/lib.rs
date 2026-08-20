//! # sysml-codegen
//!
//! Code generation utilities for the sysml-rs project.
//!
//! This crate provides tools to generate Rust code from SysML v2 specification files,
//! particularly the TTL (Turtle) vocabulary files that define element types and
//! OSLC shapes files that define property constraints.
//!
//! ## Overview
//!
//! The main use cases are:
//! - Generating the `ElementKind` enum from KerML and SysML vocabularies
//! - Generating typed property accessors from OSLC shapes
//! - Generating validation methods based on shape constraints
//!
//! ## Usage in build.rs
//!
//! ```ignore
//! // In sysml-core/build.rs
//! use sysml_codegen::{parse_ttl_vocab, generate_enum, TypeHierarchy};
//! use sysml_codegen::shapes_parser::{parse_oslc_shapes, resolve_shared_properties};
//! use sysml_codegen::inheritance::resolve_inheritance;
//! use sysml_codegen::accessor_generator::generate_property_accessors;
//! use std::fs;
//! use std::path::Path;
//!
//! fn main() {
//!     // ... vocabulary parsing ...
//!     // ... shapes parsing ...
//! }
//! ```
//!
//! ## Features
//!
//! - Parse TTL vocabulary files to extract type information
//! - Parse OSLC shapes files to extract property constraints
//! - Resolve property inheritance through type hierarchies
//! - Generate Rust enums with all SysML v2 element types
//! - Generate typed property accessor structs
//! - Generate property validation methods
//! - Include documentation comments from the specification

pub mod accessor_generator;
pub mod archetype_generator;
pub mod crossref_generator;
pub mod crossref_validation;
pub mod enum_generator;
pub mod enum_value_generator;
pub mod hierarchy_generator;
pub mod inheritance;
pub mod json_schema_parser;
pub mod property_validation_validator;
pub mod relationship_generator;
pub mod resolution_spec_validator;
pub mod semantic_rule_parser;
pub mod semantic_validation_generator;
// Bounds: character-by-character TTL parser with index guards (i < len) before every access
#[allow(clippy::indexing_slicing)]
pub mod shapes_parser;
pub mod spec_validation;
pub mod ttl_parser;
pub mod validation_generator;
pub mod xmi_class_parser;
pub mod xmi_relationship_parser;
pub mod xtext_crossref_parser;
// Bounds: line-by-line grammar parser with pre-checked positions
#[allow(clippy::indexing_slicing)]
pub mod xtext_parser;

pub use crossref_generator::generate_crossref_registry;
pub use crossref_validation::{
    infer_scope_strategy, validate_crossref_coverage, CrossRefCoverageReport, ScopeStrategy,
    INTENTIONALLY_SKIPPED,
};
pub use enum_generator::generate_enum;
pub use enum_value_generator::generate_value_enums;
pub use hierarchy_generator::{generate_hierarchy_methods, TypeHierarchy};
pub use json_schema_parser::{parse_all_enums_from_json, JsonEnumInfo};
pub use property_validation_validator::{
    validate_property_validation_coverage, ConstraintStats, ConstraintType,
    PropertyValidationCoverageResult,
};
pub use relationship_generator::{
    build_relationship_target_properties, generate_relationship_methods_with_xmi,
    generate_relationship_property_methods, get_fallback_constraint_names,
    validate_relationship_property_coverage, RelationshipConstraint,
    RelationshipPropertyCoverageReport, RelationshipTargetProperty,
};
pub use resolution_spec_validator::{
    extract_resolution_unresolved_props, is_non_crossref_prop, normalize_resolution_prop_name,
    validate_resolution_spec_completeness, validate_resolution_spec_with_mappings, ResolutionProp,
    ResolutionSpecValidationResult, NON_CROSSREF_RESOLUTION_PROPS, RESOLUTION_TO_REGISTRY_MAPPING,
};
pub use semantic_rule_parser::{
    parse_semantic_rules_file, summarize_rules, RuleCatalogSummary, SemanticRule,
};
pub use semantic_validation_generator::generate_semantic_validation_with_hierarchy;
pub use spec_validation::{
    validate_enum_coverage, validate_type_coverage, EnumCoverageReport, EnumValidation,
    TypeCoverageReport,
};
pub use ttl_parser::{
    merge_enum_info, parse_ttl_enums, parse_ttl_vocab, EnumInfo, EnumValue, ParseError, TypeInfo,
};
pub use xmi_class_parser::parse_all_xmi_classes;
pub use xmi_relationship_parser::{
    parse_relationship_constraints, validate_relationship_coverage, CoverageReport, XmiParseError,
    XmiRelationshipConstraint,
};
pub use xtext_crossref_parser::{parse_xtext_cross_references, CrossReference};
pub use xtext_parser::{
    extract_all_keyword_strings, parse_grammar_ir, parse_grammar_ir_body, parse_xtext_enums,
    parse_xtext_operators, parse_xtext_rules, GrammarRuleIr, IrNode, OperatorInfo, XtextEnumInfo,
    XtextRule,
};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn integration_test() {
        // Test the full pipeline
        let kerml_ttl = r#"
@prefix oslc_kerml: <https://www.omg.org/spec/kerml/vocabulary#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

oslc_kerml:Element a rdfs:Class ;
    rdfs:label "Element" ;
    rdfs:comment "An Element is a constituent of a model." ;
    rdfs:subClassOf oslc_am:Resource .

oslc_kerml:Relationship a rdfs:Class ;
    rdfs:label "Relationship" ;
    rdfs:subClassOf oslc_kerml:Element .
"#;

        let sysml_ttl = r#"
@prefix oslc_sysml: <https://www.omg.org/spec/sysml/vocabulary#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

oslc_sysml:PartUsage a rdfs:Class ;
    rdfs:label "PartUsage" ;
    rdfs:subClassOf oslc_sysml:ItemUsage .

oslc_sysml:ActionDefinition a rdfs:Class ;
    rdfs:label "ActionDefinition" ;
    rdfs:subClassOf oslc_sysml:Definition .
"#;

        let kerml_types = parse_ttl_vocab(kerml_ttl).unwrap();
        let sysml_types = parse_ttl_vocab(sysml_ttl).unwrap();

        assert_eq!(kerml_types.len(), 2);
        assert_eq!(sysml_types.len(), 2);

        let code = generate_enum("TestElementKind", &kerml_types, &sysml_types);

        // Verify the generated code contains expected elements
        assert!(code.contains("pub enum TestElementKind"));
        assert!(code.contains("Element,"));
        assert!(code.contains("Relationship,"));
        assert!(code.contains("PartUsage,"));
        assert!(code.contains("ActionDefinition,"));

        // Verify impl methods are generated
        assert!(code.contains("pub fn iter()"));
        assert!(code.contains("pub fn as_str(&self)"));
        assert!(code.contains("pub fn from_str(s: &str)"));
        assert!(code.contains("pub const fn count()"));
    }
}
