#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
//! Library conformance tests — verify that systems and domain library names
//! resolve correctly through the salsa LSP pipeline.
//!
//! Sprint 2 Items 7 and 8: These tests build synthetic library graphs that
//! mirror the standard SysML v2 systems library and domain library packages,
//! inject them into the LSP server, and verify that fixture files referencing
//! these libraries parse and produce correct symbols.

use sysml_core::{Element, ElementKind, ModelGraph, VisibilityKind};
use sysml_id::QualifiedName;
use tower_lsp::lsp_types::*;

use crate::test_harness::TestServer;

const TEST_URI: &str = "file:///test.sysml";

// ── Fixtures ───────────────────────────────────────────────────────

const SYSTEMS_LIBRARY_CONFORMANCE: &str =
    include_str!("../fixtures/valid/systems_library_conformance.sysml");
const DOMAIN_LIBRARY_CONFORMANCE: &str =
    include_str!("../fixtures/valid/domain_library_conformance.sysml");

// ── Library graph builders ─────────────────────────────────────────

/// Build a synthetic systems library graph containing the standard SysML v2
/// systems library packages: Requirements, Actions, States, Parts, etc.
///
/// Each package contains a representative definition so that name resolution
/// can find something meaningful beyond just the package namespace.
fn build_systems_library_graph() -> ModelGraph {
    let mut lib = ModelGraph::new();

    // Systems library packages that SysML v2 defines
    let packages = [
        "Requirements",
        "Actions",
        "States",
        "Parts",
        "Connections",
        "Constraints",
        "Items",
        "Ports",
        "Allocations",
        "Calculations",
        "Cases",
        "Views",
    ];

    for pkg_name in &packages {
        let pkg = Element::new_with_kind(ElementKind::LibraryPackage)
            .with_name(*pkg_name)
            .with_qname(QualifiedName::from_segments(vec![pkg_name.to_string()]));
        let pkg_id = lib.add_element(pkg);

        // Add a representative definition inside each package
        let def_name = format!("{}Def", pkg_name.trim_end_matches('s'));
        let def = Element::new_with_kind(ElementKind::Definition)
            .with_name(&def_name)
            .with_qname(QualifiedName::from_segments(vec![
                pkg_name.to_string(),
                def_name,
            ]));
        lib.add_owned_element(def, pkg_id, VisibilityKind::Public);
    }

    // Also include the ScalarValues package (commonly imported)
    let sv = Element::new_with_kind(ElementKind::LibraryPackage)
        .with_name("ScalarValues")
        .with_qname(QualifiedName::from_segments(vec![
            "ScalarValues".to_string()
        ]));
    let sv_id = lib.add_element(sv);

    for type_name in &["Integer", "Real", "String", "Boolean"] {
        let elem = Element::new_with_kind(ElementKind::AttributeDefinition)
            .with_name(*type_name)
            .with_qname(QualifiedName::from_segments(vec![
                "ScalarValues".to_string(),
                type_name.to_string(),
            ]));
        lib.add_owned_element(elem, sv_id.clone(), VisibilityKind::Public);
    }

    lib
}

/// Build a synthetic domain library graph containing ISQ and SI packages.
///
/// ISQ (International System of Quantities) defines quantity types like
/// MassValue, LengthValue, TemperatureValue.
/// SI (International System of Units) defines unit types like kilogram, metre.
fn build_domain_library_graph() -> ModelGraph {
    let mut lib = ModelGraph::new();

    // ISQ package with quantity types
    let isq = Element::new_with_kind(ElementKind::LibraryPackage)
        .with_name("ISQ")
        .with_qname(QualifiedName::from_segments(vec!["ISQ".to_string()]));
    let isq_id = lib.add_element(isq);

    for qty_name in &["MassValue", "LengthValue", "TemperatureValue", "TimeValue"] {
        let elem = Element::new_with_kind(ElementKind::AttributeDefinition)
            .with_name(*qty_name)
            .with_qname(QualifiedName::from_segments(vec![
                "ISQ".to_string(),
                qty_name.to_string(),
            ]));
        lib.add_owned_element(elem, isq_id.clone(), VisibilityKind::Public);
    }

    // SI package with unit types
    let si = Element::new_with_kind(ElementKind::LibraryPackage)
        .with_name("SI")
        .with_qname(QualifiedName::from_segments(vec!["SI".to_string()]));
    let si_id = lib.add_element(si);

    for unit_name in &["kilogram", "metre", "second", "kelvin"] {
        let elem = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name(*unit_name)
            .with_qname(QualifiedName::from_segments(vec![
                "SI".to_string(),
                unit_name.to_string(),
            ]));
        lib.add_owned_element(elem, si_id.clone(), VisibilityKind::Public);
    }

    // Also include ScalarValues (needed as transitive dependency)
    let sv = Element::new_with_kind(ElementKind::LibraryPackage)
        .with_name("ScalarValues")
        .with_qname(QualifiedName::from_segments(vec![
            "ScalarValues".to_string()
        ]));
    let sv_id = lib.add_element(sv);

    for type_name in &["Integer", "Real", "String", "Boolean"] {
        let elem = Element::new_with_kind(ElementKind::AttributeDefinition)
            .with_name(*type_name)
            .with_qname(QualifiedName::from_segments(vec![
                "ScalarValues".to_string(),
                type_name.to_string(),
            ]));
        lib.add_owned_element(elem, sv_id.clone(), VisibilityKind::Public);
    }

    lib
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Collect all symbol names recursively from nested DocumentSymbols.
fn collect_symbol_names(symbols: &[DocumentSymbol]) -> Vec<String> {
    let mut names = Vec::new();
    for sym in symbols {
        names.push(sym.name.clone());
        if let Some(ref children) = sym.children {
            names.extend(collect_symbol_names(children));
        }
    }
    names
}

// ════════════════════════════════════════════════════════════════════
// Item 7: Systems library conformance
// ════════════════════════════════════════════════════════════════════

/// Verify that the systems library conformance fixture parses without errors
/// when the systems library graph is loaded.
#[tokio::test]
async fn systems_library_fixture_parses_with_library() {
    let server = TestServer::new();
    server.initialize_full().await;
    server
        .set_library_graph(build_systems_library_graph())
        .await;

    server
        .open_document(TEST_URI, SYSTEMS_LIBRARY_CONFORMANCE)
        .await;

    // The fixture should produce document symbols (not crash or produce empty results)
    let response = server.document_symbol(TEST_URI).await;
    let symbols = match response {
        Some(DocumentSymbolResponse::Nested(syms)) => syms,
        _ => panic!("Expected nested symbols for systems library fixture"),
    };

    let names = collect_symbol_names(&symbols);

    // The top-level package should be present
    assert!(
        names.contains(&"SystemsLibraryConformance".to_string()),
        "Should find top-level package 'SystemsLibraryConformance', got: {:?}",
        names
    );

    // Definitions inside the fixture should be present
    assert!(
        names.contains(&"SafetyReq".to_string()),
        "Should find 'SafetyReq' requirement definition, got: {:?}",
        names
    );
    assert!(
        names.contains(&"DriveAction".to_string()),
        "Should find 'DriveAction' action definition, got: {:?}",
        names
    );
    assert!(
        names.contains(&"OperatingState".to_string()),
        "Should find 'OperatingState' state definition, got: {:?}",
        names
    );
    assert!(
        names.contains(&"Vehicle".to_string()),
        "Should find 'Vehicle' part definition, got: {:?}",
        names
    );
    assert!(
        names.contains(&"MassLimit".to_string()),
        "Should find 'MassLimit' constraint definition, got: {:?}",
        names
    );
}

/// Verify that systems library package names are available for completion
/// when the library graph is loaded.
#[tokio::test]
async fn systems_library_completion_includes_library_packages() {
    let server = TestServer::new();
    server.initialize_full().await;
    server
        .set_library_graph(build_systems_library_graph())
        .await;

    // Open a file with a partial import statement
    server
        .open_document(TEST_URI, "package P {\n  import Req\n}")
        .await;

    let response = server.completion(TEST_URI, 1, 12, None).await;

    let items = match response {
        Some(CompletionResponse::List(list)) => list.items,
        Some(CompletionResponse::Array(items)) => items,
        None => vec![],
    };

    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();

    // With the systems library loaded, "Requirements" should appear
    assert!(
        labels.contains(&"Requirements"),
        "With systems library loaded, completion for 'Req' should include 'Requirements', got: {:?}",
        labels
    );
}

/// Verify that the fixture produces semantic tokens (proving the file parses).
#[tokio::test]
async fn systems_library_fixture_produces_semantic_tokens() {
    let server = TestServer::new();
    server.initialize_full().await;
    server
        .set_library_graph(build_systems_library_graph())
        .await;

    server
        .open_document(TEST_URI, SYSTEMS_LIBRARY_CONFORMANCE)
        .await;

    let result = server.semantic_tokens_full(TEST_URI).await;
    let tokens = match result {
        Some(SemanticTokensResult::Tokens(t)) => t.data,
        Some(SemanticTokensResult::Partial(p)) => p.data,
        None => vec![],
    };

    assert!(
        !tokens.is_empty(),
        "Systems library conformance fixture should produce semantic tokens"
    );
}

// ════════════════════════════════════════════════════════════════════
// Item 8: Domain library conformance
// ════════════════════════════════════════════════════════════════════

/// Verify that the domain library conformance fixture parses without errors
/// when the ISQ/SI library graph is loaded.
#[tokio::test]
async fn domain_library_fixture_parses_with_library() {
    let server = TestServer::new();
    server.initialize_full().await;
    server.set_library_graph(build_domain_library_graph()).await;

    server
        .open_document(TEST_URI, DOMAIN_LIBRARY_CONFORMANCE)
        .await;

    // The fixture should produce document symbols
    let response = server.document_symbol(TEST_URI).await;
    let symbols = match response {
        Some(DocumentSymbolResponse::Nested(syms)) => syms,
        _ => panic!("Expected nested symbols for domain library fixture"),
    };

    let names = collect_symbol_names(&symbols);

    // The top-level package should be present
    assert!(
        names.contains(&"DomainLibraryConformance".to_string()),
        "Should find top-level package 'DomainLibraryConformance', got: {:?}",
        names
    );

    // Definitions inside the fixture should be present
    assert!(
        names.contains(&"PhysicalSensor".to_string()),
        "Should find 'PhysicalSensor' part definition, got: {:?}",
        names
    );
    assert!(
        names.contains(&"MeasuredComponent".to_string()),
        "Should find 'MeasuredComponent' part definition, got: {:?}",
        names
    );
}

/// Verify that ISQ/SI package names are available for completion.
#[tokio::test]
async fn domain_library_completion_includes_isq_si() {
    let server = TestServer::new();
    server.initialize_full().await;
    server.set_library_graph(build_domain_library_graph()).await;

    // Open a file with a partial import statement for ISQ
    server
        .open_document(TEST_URI, "package P {\n  import IS\n}")
        .await;

    let response = server.completion(TEST_URI, 1, 11, None).await;

    let items = match response {
        Some(CompletionResponse::List(list)) => list.items,
        Some(CompletionResponse::Array(items)) => items,
        None => vec![],
    };

    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();

    // With the domain library loaded, "ISQ" should appear
    assert!(
        labels.contains(&"ISQ"),
        "With domain library loaded, completion for 'IS' should include 'ISQ', got: {:?}",
        labels
    );
}

/// Verify that the domain library fixture produces semantic tokens.
#[tokio::test]
async fn domain_library_fixture_produces_semantic_tokens() {
    let server = TestServer::new();
    server.initialize_full().await;
    server.set_library_graph(build_domain_library_graph()).await;

    server
        .open_document(TEST_URI, DOMAIN_LIBRARY_CONFORMANCE)
        .await;

    let result = server.semantic_tokens_full(TEST_URI).await;
    let tokens = match result {
        Some(SemanticTokensResult::Tokens(t)) => t.data,
        Some(SemanticTokensResult::Partial(p)) => p.data,
        None => vec![],
    };

    assert!(
        !tokens.is_empty(),
        "Domain library conformance fixture should produce semantic tokens"
    );
}

/// Verify that ISQ quantity types appear in attribute type position completions.
#[tokio::test]
async fn domain_library_isq_types_in_attribute_completion() {
    let server = TestServer::new();
    server.initialize_full().await;
    server.set_library_graph(build_domain_library_graph()).await;

    // Open a file with partial ISQ type reference
    server
        .open_document(
            TEST_URI,
            "package P {\n  import ISQ::*;\n  part def S {\n    attribute m : Mass\n  }\n}",
        )
        .await;

    let response = server.completion(TEST_URI, 3, 22, None).await;

    let items = match response {
        Some(CompletionResponse::List(list)) => list.items,
        Some(CompletionResponse::Array(items)) => items,
        None => vec![],
    };

    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();

    // With ISQ namespace imported, MassValue should appear
    assert!(
        labels.contains(&"MassValue"),
        "With ISQ imported, completion for 'Mass' should include 'MassValue', got: {:?}",
        labels
    );
}
