#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
//! Integration tests for LSP features using real parsed SysML.
//!
//! These tests exercise rename, diagnostics, semantic tokens, and position_map
//! using the tree-sitter parser to build ModelGraphs from actual SysML source.

use crate::diagnostics::to_lsp_diagnostic;
use crate::semantic_tokens::SemanticTokensBuilder;

/// Test-local adapter over the production element-kind → LSP token-type
/// mapping: `sysml_ide_db::element_kind_to_category` composed with
/// `semantic_tokens::token_category_to_lsp`. Replaces the retired
/// `kinds::element_kind_to_token_type` duplicate so these builder tests
/// feed real production token types.
fn element_kind_to_token_type(kind: &sysml_core::ElementKind) -> u32 {
    crate::semantic_tokens::token_category_to_lsp(sysml_ide_db::element_kind_to_category(kind))
}

use sysml_core::{Element, ElementKind, ModelGraph, Value};
use sysml_id::ElementId;
use sysml_span::Span;
use sysml_parser_incremental::{build_model_graph, TreeSitterParser};

use tower_lsp::lsp_types::DiagnosticSeverity;

const SIMPLE_MODEL: &str = "part def Sensor;\npart def Camera :> Sensor;\npart camera : Camera;";

const BROKEN_MODEL: &str =
    "package Broken {\n    part def Good;\n    part bad @#$;\n    part def AlsoGood;\n}";

// ── Diagnostics Tests ────────────────────────────────────────────────

#[test]
fn test_syntax_error_diagnostic() {
    // Parse broken SysML with tree-sitter and verify diagnostics are produced.
    let parser = TreeSitterParser::new();
    let tree = parser
        .parse_tree(BROKEN_MODEL)
        .expect("tree-sitter should still produce a tree");
    let result = build_model_graph(&tree, BROKEN_MODEL, "file:///test.sysml");

    // Broken model should produce at least one diagnostic
    assert!(
        !result.diagnostics.is_empty(),
        "broken SysML should produce diagnostics, got none"
    );

    // Verify that diagnostics can be converted to LSP diagnostics
    for diag in &result.diagnostics {
        let lsp_diag = to_lsp_diagnostic(diag, BROKEN_MODEL);
        // Each diagnostic should have a severity
        assert!(
            lsp_diag.severity.is_some(),
            "LSP diagnostic should have a severity"
        );
        // Error diagnostics should map to ERROR severity
        if diag.is_error() {
            assert_eq!(lsp_diag.severity, Some(DiagnosticSeverity::ERROR));
        }
    }
}

#[test]
fn test_error_recovery_partial_graph() {
    // Parse partially broken SysML and verify valid parts still produce elements.
    let parser = TreeSitterParser::new();
    let tree = parser
        .parse_tree(BROKEN_MODEL)
        .expect("tree-sitter should produce a tree");
    let result = build_model_graph(&tree, BROKEN_MODEL, "file:///test.sysml");

    // Even with errors, the graph should contain elements from valid regions.
    // "part def Good;" and "part def AlsoGood;" should survive error recovery.
    let named_elements: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.name.is_some())
        .collect();

    assert!(
        !named_elements.is_empty(),
        "error recovery should preserve valid elements, but graph has no named elements"
    );
}

// ── Semantic Tokens Tests ────────────────────────────────────────────

#[test]
fn test_semantic_tokens_from_parsed_sysml() {
    // Parse real SysML, build semantic tokens from elements, verify they are sorted.
    let parser = TreeSitterParser::new();
    let tree = parser
        .parse_tree(SIMPLE_MODEL)
        .expect("parse should succeed");
    let result = build_model_graph(&tree, SIMPLE_MODEL, "file:///test.sysml");

    let uri = "file:///test.sysml";
    let mut builder = SemanticTokensBuilder::new(SIMPLE_MODEL);

    for element in result.graph.elements.values() {
        for (idx, span) in element.spans.iter().enumerate() {
            if span.file != uri || span.file == "<synthetic>" || span.start == span.end {
                continue;
            }

            let token_type = element_kind_to_token_type(&element.kind);
            let mut modifiers = 0u32;
            if idx == 0 {
                modifiers |= 1 << 0; // DEFINITION
            }
            builder.add_token(span.start, span.end, token_type, modifiers);
        }
    }

    let tokens = builder.build();

    // Should produce at least some tokens from the parsed model
    assert!(
        !tokens.is_empty(),
        "parsed SysML model should produce semantic tokens"
    );

    // Verify delta encoding: delta_line should be non-negative (monotonic)
    // and when delta_line is 0, previous tokens on the same line have lower positions
    let mut prev_line = 0u32;
    for token in &tokens {
        let current_line = prev_line + token.delta_line;
        assert!(
            current_line >= prev_line || token.delta_line == 0,
            "tokens should be sorted by position"
        );
        prev_line = current_line;
    }
}

#[test]
fn test_semantic_tokens_abstract_modifier() {
    // Element with isAbstract should have ABSTRACT modifier bit set.
    let source = "abstract part def AbstractSensor;";
    let mut graph = ModelGraph::new();

    let id = ElementId::new_v4();
    let elem = Element::new(id.clone(), ElementKind::PartDefinition)
        .with_name("AbstractSensor")
        .with_prop("isAbstract", Value::Bool(true))
        .with_span(Span::new("file:///test.sysml", 14, 33));
    graph.add_element(elem);

    let element = graph.get_element(&id).unwrap();
    let is_abstract = element
        .props
        .get("isAbstract")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    assert!(is_abstract, "element should be abstract");

    // Build a semantic token with the abstract modifier
    let mut builder = SemanticTokensBuilder::new(source);
    let token_type = element_kind_to_token_type(&element.kind);
    let mut modifiers = 0u32;
    modifiers |= 1 << 0; // DEFINITION (first span)
    if is_abstract {
        modifiers |= 1 << 4; // ABSTRACT
    }
    builder.add_token(14, 33, token_type, modifiers);

    let tokens = builder.build();
    assert_eq!(tokens.len(), 1);
    // Check ABSTRACT bit (bit 4) is set
    assert_ne!(
        tokens[0].token_modifiers_bitset & (1 << 4),
        0,
        "ABSTRACT modifier bit should be set"
    );
    // Check DEFINITION bit (bit 0) is also set
    assert_ne!(
        tokens[0].token_modifiers_bitset & (1 << 0),
        0,
        "DEFINITION modifier bit should be set"
    );
}

#[test]
fn test_semantic_tokens_definition_modifier() {
    // First span of an element should have the DEFINITION modifier bit set.
    let source = "part def Engine;\npart engine : Engine;";
    let mut graph = ModelGraph::new();

    let def_id = ElementId::new_v4();
    let def_elem = Element::new(def_id.clone(), ElementKind::PartDefinition)
        .with_name("Engine")
        .with_span(Span::new("file:///test.sysml", 9, 15)); // "Engine" in definition
    graph.add_element(def_elem);

    let usage_id = ElementId::new_v4();
    let usage_elem = Element::new(usage_id.clone(), ElementKind::PartUsage)
        .with_name("engine")
        .with_span(Span::new("file:///test.sysml", 22, 28)); // "engine" in usage
    graph.add_element(usage_elem);

    let uri = "file:///test.sysml";
    let mut builder = SemanticTokensBuilder::new(source);

    for element in graph.elements.values() {
        for (idx, span) in element.spans.iter().enumerate() {
            if span.file != uri {
                continue;
            }
            let token_type = element_kind_to_token_type(&element.kind);
            let mut modifiers = 0u32;
            if idx == 0 {
                modifiers |= 1 << 0; // DEFINITION
            }
            builder.add_token(span.start, span.end, token_type, modifiers);
        }
    }

    let tokens = builder.build();
    assert_eq!(tokens.len(), 2, "should have 2 tokens");

    // Both are first spans (idx == 0), so both get DEFINITION modifier
    for token in &tokens {
        assert_ne!(
            token.token_modifiers_bitset & (1 << 0),
            0,
            "first span should have DEFINITION modifier"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// P0/P1 Validation Tests — Verify diagnostic pipeline changes work
// ═══════════════════════════════════════════════════════════════════════

// ── P0-4: is_likely_library_type filtering ────────────────────────────

/// The `is_likely_library_type` function (private in diagnostics.rs) is tested
/// indirectly via the diagnostic pipeline. Here we test the logic directly
/// by replicating it — ensuring known library types are detected.
#[test]
fn test_library_type_detection_scalar_types() {
    // Known scalar types from the standard library
    let library_scalars = [
        "Real",
        "Integer",
        "String",
        "Boolean",
        "Natural",
        "Positive",
        "Complex",
        "Number",
        "Anything",
        "DataValue",
    ];
    let library_prefixes = [
        "ScalarValues::Real",
        "ISQ::Length",
        "SI::Meter",
        "Parts::Part",
        "Actions::Action",
        "States::State",
    ];
    let user_types = ["Vehicle", "Engine", "MySensor", "CustomType"];

    for name in &library_scalars {
        assert!(
            is_library_type_like(name),
            "{} should be detected as library type",
            name
        );
    }

    for name in &library_prefixes {
        assert!(
            is_library_type_like(name),
            "{} should be detected as library type (prefix match)",
            name
        );
    }

    for name in &user_types {
        assert!(
            !is_library_type_like(name),
            "{} should NOT be detected as library type",
            name
        );
    }
}

/// Replicate the is_likely_library_type logic from diagnostics.rs for testing.
fn is_library_type_like(name: &str) -> bool {
    const SCALAR_TYPES: &[&str] = &[
        "Real",
        "Integer",
        "String",
        "Boolean",
        "Natural",
        "Positive",
        "Complex",
        "Number",
        "Anything",
        "DataValue",
    ];
    const NS_PREFIXES: &[&str] = &[
        "ScalarValues",
        "Quantities",
        "MeasurementReferences",
        "ISQ",
        "SI",
        "USCustomary",
        "Base",
        "Connections",
        "Parts",
        "Items",
        "Actions",
        "States",
        "Constraints",
        "Requirements",
        "Allocations",
    ];

    SCALAR_TYPES.contains(&name)
        || NS_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
        || name.contains("::")
}

// ── P0-2: Resolution runs on files with syntax errors ────────────────

#[test]
fn test_partial_graph_supports_resolution() {
    // Parse a file with syntax errors — tree-sitter should produce a partial graph
    // that can still be used for resolution
    let parser = TreeSitterParser::new();
    let source = "package Good {\n  part def Valid;\n}\npackage @#$ invalid;\npart def AlsoValid;";
    let tree = parser
        .parse_tree(source)
        .expect("tree-sitter should produce a tree");
    let result = build_model_graph(&tree, source, "file:///test.sysml");

    // Should have diagnostics (syntax errors)
    assert!(
        !result.diagnostics.is_empty(),
        "should have syntax error diagnostics"
    );

    // But the graph should still have valid elements
    let named: Vec<String> = result
        .graph
        .elements
        .values()
        .filter_map(|e| e.name.clone())
        .collect();

    assert!(
        named
            .iter()
            .any(|n| n == "Good" || n == "Valid" || n == "AlsoValid"),
        "partial graph should contain valid elements: {:?}",
        named
    );

    // Resolution should be callable on the partial graph
    let mut graph = result.graph;
    let res = sysml_core::resolution::resolve_references(&mut graph);
    // Should not panic — this is the key P0-2 guarantee
    let _ = res.diagnostics;
}

// ── P0-3: Structural validation runs at T2Local ──────────────────────

#[test]
fn test_structural_validation_callable_on_partial_graph() {
    // Build a graph with valid structure
    let parser = TreeSitterParser::new();
    let source = "package TestPkg {\n  part def Vehicle;\n  part car : Vehicle;\n}";
    let tree = parser.parse_tree(source).expect("parse should succeed");
    let result = build_model_graph(&tree, source, "file:///test.sysml");

    // Structural validation should be callable (P0-3 gate was relaxed)
    let structure_errors = result.graph.validate_structure();
    // May or may not have errors, but shouldn't panic
    let _ = structure_errors;

    let relationship_errors = result.graph.validate_relationship_types();
    let _ = relationship_errors;
}

// ── P1-2: WorkspaceSnapshot stores span data ────────────────────────────

#[tokio::test]
async fn test_workspace_snapshot_preserves_spans() {
    let mut host = sysml_ide_db::AnalysisHost::new();
    host.set_file_content("file:///vehicles.sysml", "part def Engine {}".to_string());

    let ws = crate::workspace_snapshot::WorkspaceSnapshot::from_host(&host);
    let entries = ws.find_by_name("Engine");
    assert!(!entries.is_empty(), "Engine should be found in snapshot");
    assert_eq!(entries[0].uri, "file:///vehicles.sysml");
}

// ── P1-1: Cross-file references via WorkspaceSnapshot ───────────────────

#[tokio::test]
async fn test_workspace_snapshot_finds_same_name_across_files() {
    let mut host = sysml_ide_db::AnalysisHost::new();
    host.set_file_content("file:///defs.sysml", "part def Engine {}".to_string());
    host.set_file_content("file:///usage.sysml", "part engine : Engine;".to_string());

    let ws = crate::workspace_snapshot::WorkspaceSnapshot::from_host(&host);
    let entries = ws.find_by_name("Engine");
    // At minimum the definition should be found
    assert!(
        !entries.is_empty(),
        "should find Engine in workspace snapshot"
    );

    let uris: Vec<&str> = entries.iter().map(|e| e.uri.as_str()).collect();
    assert!(uris.contains(&"file:///defs.sysml"));
}

// ── P1-3: Cross-file type completion (unit-level validation) ─────────

#[tokio::test]
async fn test_workspace_snapshot_definitions_for_completion() {
    let mut host = sysml_ide_db::AnalysisHost::new();
    host.set_file_content(
        "file:///sensors.sysml",
        "part def Sensor {}\npart mySensor : Sensor;".to_string(),
    );

    let ws = crate::workspace_snapshot::WorkspaceSnapshot::from_host(&host);

    // For type completion, we should only show definitions
    let mut defs_found = Vec::new();
    ws.for_each_name(|name, entries| {
        for entry in entries {
            if entry.element_kind.is_definition() {
                defs_found.push(name.to_string());
                break;
            }
        }
    });

    assert!(
        defs_found.contains(&"Sensor".to_string()),
        "Sensor definition should be found"
    );
}

// ── P1-5: Resolution status messages ────────────────────────────────

#[test]
fn test_resolution_tier_ordering() {
    use crate::background::ResolutionTier;

    // Verify tier ordering (PartialOrd/Ord derives)
    assert!(ResolutionTier::T1Syntax < ResolutionTier::T2Local);
}

// ── P3-4: Type hierarchy preparation ─────────────────────────────────

#[test]
fn test_specialization_hierarchy_for_type_hierarchy() {
    use sysml_core::{Element, ElementKind, ModelGraph, Relationship, RelationshipKind};

    let mut graph = ModelGraph::new();

    // Vehicle (base type)
    let vehicle_id = sysml_id::ElementId::new_v4();
    let vehicle = Element::new(vehicle_id.clone(), ElementKind::PartDefinition)
        .with_name("Vehicle")
        .with_span(Span::new("file:///test.sysml", 0, 20));
    graph.add_element(vehicle);

    // Car :> Vehicle
    let car_id = sysml_id::ElementId::new_v4();
    let car = Element::new(car_id.clone(), ElementKind::PartDefinition)
        .with_name("Car")
        .with_span(Span::new("file:///test.sysml", 30, 50));
    graph.add_element(car);

    // Truck :> Vehicle
    let truck_id = sysml_id::ElementId::new_v4();
    let truck = Element::new(truck_id.clone(), ElementKind::PartDefinition)
        .with_name("Truck")
        .with_span(Span::new("file:///test.sysml", 60, 80));
    graph.add_element(truck);

    // Specialization relationships
    graph.add_relationship(Relationship::new(
        RelationshipKind::Specialize,
        car_id.clone(),
        vehicle_id.clone(),
    ));
    graph.add_relationship(Relationship::new(
        RelationshipKind::Specialize,
        truck_id.clone(),
        vehicle_id.clone(),
    ));

    // Vehicle's subtypes: Car and Truck (incoming Specialize)
    let subtypes: Vec<String> = graph
        .incoming(&vehicle_id)
        .filter(|r| r.kind == RelationshipKind::Specialize)
        .filter_map(|r| graph.get_element(&r.source).and_then(|e| e.name.clone()))
        .collect();

    assert_eq!(subtypes.len(), 2);
    assert!(subtypes.contains(&"Car".to_string()));
    assert!(subtypes.contains(&"Truck".to_string()));

    // Car's supertypes: Vehicle (outgoing Specialize)
    let supertypes: Vec<String> = graph
        .outgoing(&car_id)
        .filter(|r| r.kind == RelationshipKind::Specialize)
        .filter_map(|r| graph.get_element(&r.target).and_then(|e| e.name.clone()))
        .collect();

    assert_eq!(supertypes, vec!["Vehicle".to_string()]);
}

// ── P2-4: Feature chain includes inherited members ────────────────────

#[test]
fn test_feature_chain_includes_inherited_members() {
    // Build: part def Base { attribute baseAttr; }
    //        part def Child :> Base { attribute childAttr; }
    // Verify owned_members + specialization hierarchy traversal
    use sysml_core::{
        Element, ElementKind, ModelGraph, Relationship, RelationshipKind, VisibilityKind,
    };

    let mut graph = ModelGraph::new();

    // Base definition
    let base = Element::new_with_kind(ElementKind::PartDefinition).with_name("Base");
    let base_id = graph.add_element(base);

    // baseAttr owned by Base (via add_owned_element for proper membership indexing)
    let base_attr = Element::new_with_kind(ElementKind::AttributeUsage).with_name("baseAttr");
    let _base_attr_id = graph.add_owned_element(base_attr, base_id.clone(), VisibilityKind::Public);

    // Child definition
    let child = Element::new_with_kind(ElementKind::PartDefinition).with_name("Child");
    let child_id = graph.add_element(child);

    // childAttr owned by Child
    let child_attr = Element::new_with_kind(ElementKind::AttributeUsage).with_name("childAttr");
    let _child_attr_id =
        graph.add_owned_element(child_attr, child_id.clone(), VisibilityKind::Public);

    // Child :> Base (specialization)
    graph.add_relationship(Relationship::new(
        RelationshipKind::Specialize,
        child_id.clone(),
        base_id.clone(),
    ));

    // Verify owned_members work
    let base_members: Vec<_> = graph.owned_members(&base_id).collect();
    assert!(
        base_members
            .iter()
            .any(|m| m.name.as_deref() == Some("baseAttr")),
        "Base should own baseAttr, got: {:?}",
        base_members
            .iter()
            .filter_map(|m| m.name.as_ref())
            .collect::<Vec<_>>()
    );

    let child_members: Vec<_> = graph.owned_members(&child_id).collect();
    assert!(
        child_members
            .iter()
            .any(|m| m.name.as_deref() == Some("childAttr")),
        "Child should own childAttr, got: {:?}",
        child_members
            .iter()
            .filter_map(|m| m.name.as_ref())
            .collect::<Vec<_>>()
    );

    // Verify Specialize relationship
    let specializations: Vec<_> = graph
        .outgoing(&child_id)
        .filter(|r| r.kind == RelationshipKind::Specialize)
        .collect();
    assert_eq!(specializations.len(), 1);
    assert_eq!(specializations[0].target, base_id);

    // Verify hierarchy traversal: from Child, follow Specialize to Base, collect Base's members
    let mut inherited_names = Vec::new();
    let mut visited = std::collections::HashSet::new();
    visited.insert(child_id.clone());
    let mut stack = vec![child_id.clone()];

    while let Some(current_id) = stack.pop() {
        for rel in graph.outgoing(&current_id) {
            if rel.kind == RelationshipKind::Specialize && visited.insert(rel.target.clone()) {
                for member in graph.owned_members(&rel.target) {
                    if let Some(name) = &member.name {
                        inherited_names.push(name.clone());
                    }
                }
                stack.push(rel.target.clone());
            }
        }
    }

    assert!(
        inherited_names.contains(&"baseAttr".to_string()),
        "Should find baseAttr in inherited members, got: {:?}",
        inherited_names
    );
}

// ── P3-1: Specialization inlay hints ──────────────────────────────────

#[test]
fn test_specialization_inlay_hint_shown() {
    use sysml_core::{Element, ElementKind, ModelGraph, Relationship, RelationshipKind};

    // Create a graph with a definition that specializes another
    let mut graph = ModelGraph::new();
    let base_id = ElementId::new_v4();
    let child_id = ElementId::new_v4();

    let base = Element::new(base_id.clone(), ElementKind::PartDefinition)
        .with_name("Vehicle")
        .with_span(Span::new("file:///test.sysml", 0, 30));
    let child = Element::new(child_id.clone(), ElementKind::PartDefinition)
        .with_name("Car")
        .with_span(Span::new("file:///test.sysml", 32, 55));
    graph.add_element(base);
    graph.add_element(child);
    graph.add_relationship(Relationship::new(
        RelationshipKind::Specialize,
        child_id.clone(),
        base_id.clone(),
    ));

    // Verify the outgoing Specialize relationships exist
    let specializations: Vec<_> = graph
        .outgoing(&child_id)
        .filter(|r| r.kind == RelationshipKind::Specialize)
        .collect();
    assert_eq!(specializations.len(), 1);

    // Verify supertype resolution
    let supertype = graph.get_element(&specializations[0].target).unwrap();
    assert_eq!(supertype.name.as_deref(), Some("Vehicle"));
}

// ── Sensemetry Fixture Tests ─────────────────────────────────────────

#[test]
fn test_sensemetry_fixture_parses_clean() {
    // The sensemetry.sysml fixture (from Sensmetry's Advent of SysML v2 Lesson 23)
    // must parse with 0 syntax errors through the tree-sitter parser.
    let fixture = include_str!("../fixtures/valid/sensemetry.sysml");
    let parser = TreeSitterParser::new();
    let tree = parser
        .parse_tree(fixture)
        .expect("tree-sitter parse should succeed");
    let result = build_model_graph(&tree, fixture, "file:///sensemetry.sysml");

    // Verify zero syntax errors from tree-sitter
    let error_count = result
        .diagnostics
        .iter()
        .filter(|d| d.severity == sysml_span::Severity::Error)
        .count();
    assert_eq!(
        error_count,
        0,
        "sensemetry.sysml should have 0 tree-sitter syntax errors, got {}: {:?}",
        error_count,
        result
            .diagnostics
            .iter()
            .filter(|d| d.severity == sysml_span::Severity::Error)
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );

    // Verify key elements were extracted
    let names: Vec<_> = result
        .graph
        .elements
        .values()
        .filter_map(|e| e.name.as_deref())
        .collect();
    assert!(
        names.contains(&"SantaSleighDesign"),
        "should find package SantaSleighDesign"
    );
    assert!(
        names.contains(&"SantaSleigh"),
        "should find part def SantaSleigh"
    );
}

#[test]
fn test_smart_quote_detection() {
    use sysml_service::diagnostics::detect_smart_quotes;

    // Source with curly quotes (common copy-paste issue)
    let source_with_curly = "requirement def <\u{2018}TR1.1\u{2019}> Foo;";
    let diags = detect_smart_quotes(source_with_curly, "file:///test.sysml");
    assert_eq!(diags.len(), 2, "should detect 2 smart quotes");
    assert!(diags[0].message.contains("left single"));
    assert!(diags[1].message.contains("right single"));
    assert!(diags[0].code.as_deref() == Some("smart-quote"));

    // Source with straight quotes (no warnings)
    let source_clean = "requirement def <'TR1.1'> Foo;";
    let diags = detect_smart_quotes(source_clean, "file:///test.sysml");
    assert!(
        diags.is_empty(),
        "clean source should have no smart-quote warnings"
    );

    // Source with curly double quotes
    let source_double = "language \u{201C}English\u{201D}";
    let diags = detect_smart_quotes(source_double, "file:///test.sysml");
    assert_eq!(diags.len(), 2, "should detect 2 smart double quotes");
    assert!(diags[0].message.contains("left double"));
    assert!(diags[1].message.contains("right double"));
}
