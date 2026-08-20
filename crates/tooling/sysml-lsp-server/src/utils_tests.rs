#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
//! Unit tests for utility functions: scoring, fuzzy matching, identifier validation.

use crate::utils::{fuzzy_match, score_completion};
use sysml_service::rename::validate_identifier;

// --- score_completion ---

#[test]
fn score_exact_match() {
    assert_eq!(score_completion("Vehicle", "Vehicle"), 100);
}

#[test]
fn score_case_insensitive_exact() {
    assert_eq!(score_completion("vehicle", "Vehicle"), 80);
    assert_eq!(score_completion("VEHICLE", "Vehicle"), 80);
}

#[test]
fn score_prefix_match() {
    assert_eq!(score_completion("Veh", "Vehicle"), 60);
}

#[test]
fn score_case_insensitive_prefix() {
    assert_eq!(score_completion("veh", "Vehicle"), 40);
}

#[test]
fn score_fuzzy_match() {
    assert_eq!(score_completion("vcl", "Vehicle"), 20);
}

#[test]
fn score_no_match() {
    assert_eq!(score_completion("xyz", "Vehicle"), 0);
}

#[test]
fn score_empty_query() {
    assert_eq!(score_completion("", "Vehicle"), 50);
    assert_eq!(score_completion("", "anything"), 50);
}

#[test]
fn score_ordering_is_consistent() {
    // More specific matches should score higher
    let exact = score_completion("part", "part");
    let prefix = score_completion("par", "part");
    let fuzzy = score_completion("pt", "part");
    assert!(exact > prefix);
    assert!(prefix > fuzzy);
}

// --- fuzzy_match ---

#[test]
fn fuzzy_substring_match() {
    // fuzzy_match is case-sensitive; callers lowercase before calling
    assert!(fuzzy_match("def", "partdefinition"));
    assert!(fuzzy_match("def", "definition"));
}

#[test]
fn fuzzy_subsequence_match() {
    assert!(fuzzy_match("pdef", "partdefinition"));
    assert!(fuzzy_match("vcl", "vehicle"));
}

#[test]
fn fuzzy_no_match() {
    assert!(!fuzzy_match("xyz", "vehicle"));
    assert!(!fuzzy_match("zzz", "abc"));
}

#[test]
fn fuzzy_empty_query() {
    assert!(fuzzy_match("", "anything"));
}

#[test]
fn fuzzy_query_longer_than_target() {
    assert!(!fuzzy_match("longquery", "short"));
}

#[test]
fn fuzzy_exact_match() {
    assert!(fuzzy_match("part", "part"));
}

// --- validate_identifier ---

#[test]
fn validate_valid_identifiers() {
    assert!(validate_identifier("Vehicle").is_ok());
    assert!(validate_identifier("_private").is_ok());
    assert!(validate_identifier("myPart123").is_ok());
    assert!(validate_identifier("A").is_ok());
    assert!(validate_identifier("_").is_ok());
}

#[test]
fn validate_empty_name() {
    let err = validate_identifier("").unwrap_err();
    assert!(err.contains("empty"));
}

#[test]
fn validate_starts_with_number() {
    let err = validate_identifier("123abc").unwrap_err();
    assert!(err.contains("must start with"));
}

#[test]
fn validate_starts_with_special_char() {
    let err = validate_identifier("@name").unwrap_err();
    assert!(err.contains("must start with"));
}

#[test]
fn validate_contains_invalid_char() {
    let err = validate_identifier("my-part").unwrap_err();
    assert!(err.contains("invalid character"));

    let err = validate_identifier("my.part").unwrap_err();
    assert!(err.contains("invalid character"));

    let err = validate_identifier("my part").unwrap_err();
    assert!(err.contains("invalid character"));
}

#[test]
fn validate_reserved_keywords() {
    let err = validate_identifier("package").unwrap_err();
    assert!(err.contains("reserved"));

    let err = validate_identifier("part").unwrap_err();
    assert!(err.contains("reserved"));

    let err = validate_identifier("import").unwrap_err();
    assert!(err.contains("reserved"));

    let err = validate_identifier("abstract").unwrap_err();
    assert!(err.contains("reserved"));
}

#[test]
fn validate_non_keyword_similar_names() {
    // These look like keywords but aren't in the reserved list
    assert!(validate_identifier("Package").is_ok()); // capitalized
    assert!(validate_identifier("parts").is_ok()); // plural
    assert!(validate_identifier("importing").is_ok()); // gerund
}
