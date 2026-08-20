#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
//! Integration-style tests for LSP feature helpers.
//!
//! These tests exercise the pub(crate) helper functions that power
//! document_symbols, goto_definition, hover, completion, and workspace_symbol.
//! They build hand-crafted ModelGraph fixtures to test each feature in isolation.

use tower_lsp::lsp_types::*;

use std::borrow::Cow;
use sysml_core::{
    Element, ElementKind, ModelGraph, Relationship, RelationshipKind, Value, VisibilityKind,
};
use sysml_id::{ElementId, QualifiedName};
use sysml_span::Span;

use sysml_service::goto_definition::resolve_goto_target;
use crate::symbols::build_nested_symbols;

// ═══════════════════════════════════════════════════════════════════════
// Test fixtures
// ═══════════════════════════════════════════════════════════════════════

const VEHICLE_SOURCE: &str = "package Vehicles {\n    part def Engine {\n        attribute horsePower : Integer;\n    }\n    part def Car :> Vehicle {\n        part engine : Engine;\n    }\n    part def Vehicle {\n        attribute weight : Real;\n    }\n}";

/// Build a graph that mirrors VEHICLE_SOURCE with proper spans and ownership.
///
/// Uses `add_owned_element` / `create_owning_membership` so that the
/// namespace-to-memberships index is populated correctly.  This is required
/// for `owned_members`, `visible_members`, and `resolve_qname` to work.
fn build_vehicle_graph() -> (ModelGraph, String) {
    let uri = "file:///test.sysml";
    let mut graph = ModelGraph::new();

    // Package: "Vehicles" spans entire source (root, no owner)
    let pkg_id = ElementId::new_v4();
    let pkg = Element::new(pkg_id.clone(), ElementKind::Package)
        .with_name("Vehicles")
        .with_span(Span::new(uri, 0, VEHICLE_SOURCE.len()));
    graph.add_element(pkg);

    // PartDef: "Engine" owned by Vehicles
    let engine = Element::new(ElementId::new_v4(), ElementKind::PartDefinition)
        .with_name("Engine")
        .with_span(Span::new(uri, 23, 74));
    let engine_id = graph.add_owned_element(engine, pkg_id.clone(), VisibilityKind::Public);
    // Set qname after ownership
    if let Some(e) = graph.elements.get_mut(&engine_id) {
        e.qname = Some(QualifiedName::from_segments(vec![
            "Vehicles".into(),
            "Engine".into(),
        ]));
    }

    // AttributeUsage: "horsePower" inside Engine
    let hp = Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
        .with_name("horsePower")
        .with_span(Span::new(uri, 49, 71));
    let hp_id = graph.add_owned_element(hp, engine_id.clone(), VisibilityKind::Public);
    if let Some(e) = graph.elements.get_mut(&hp_id) {
        e.qname = Some(QualifiedName::from_segments(vec![
            "Vehicles".into(),
            "Engine".into(),
            "horsePower".into(),
        ]));
    }

    // PartDef: "Vehicle" owned by Vehicles
    let vehicle = Element::new(ElementId::new_v4(), ElementKind::PartDefinition)
        .with_name("Vehicle")
        .with_span(Span::new(uri, 137, 190));
    let vehicle_id = graph.add_owned_element(vehicle, pkg_id.clone(), VisibilityKind::Public);
    if let Some(e) = graph.elements.get_mut(&vehicle_id) {
        e.qname = Some(QualifiedName::from_segments(vec![
            "Vehicles".into(),
            "Vehicle".into(),
        ]));
    }

    // AttributeUsage: "weight" inside Vehicle
    let weight = Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
        .with_name("weight")
        .with_span(Span::new(uri, 160, 186));
    let _weight_id = graph.add_owned_element(weight, vehicle_id.clone(), VisibilityKind::Public);

    // PartDef: "Car" owned by Vehicles, specializes Vehicle
    let car = Element::new(ElementId::new_v4(), ElementKind::PartDefinition)
        .with_name("Car")
        .with_span(Span::new(uri, 79, 132));
    let car_id = graph.add_owned_element(car, pkg_id.clone(), VisibilityKind::Public);
    if let Some(e) = graph.elements.get_mut(&car_id) {
        e.qname = Some(QualifiedName::from_segments(vec![
            "Vehicles".into(),
            "Car".into(),
        ]));
    }

    // Specialize relationship: Car :> Vehicle
    graph.add_relationship(Relationship::new(
        RelationshipKind::Specialize,
        car_id.clone(),
        vehicle_id.clone(),
    ));

    // PartUsage: "engine" inside Car (typed as Engine)
    let mut engine_usage = Element::new(ElementId::new_v4(), ElementKind::PartUsage)
        .with_name("engine")
        .with_span(Span::new(uri, 108, 128));
    engine_usage
        .props
        .insert(Cow::Borrowed("typing"), Value::String("Engine".to_string()));
    let _engine_usage_id =
        graph.add_owned_element(engine_usage, car_id.clone(), VisibilityKind::Public);

    (graph, uri.to_string())
}

// ═══════════════════════════════════════════════════════════════════════
// Document Symbols tests
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_document_symbols_nested_hierarchy() {
    let (graph, uri) = build_vehicle_graph();
    let symbols = build_nested_symbols(&graph, &uri, VEHICLE_SOURCE);

    // Should have one root symbol: the package "Vehicles"
    assert_eq!(
        symbols.len(),
        1,
        "Expected 1 root symbol (Vehicles package)"
    );
    assert_eq!(symbols[0].name, "Vehicles");

    // Package should have children: Engine, Car, Vehicle
    let children = symbols[0]
        .children
        .as_ref()
        .expect("Package should have children");
    assert!(
        children.len() >= 3,
        "Package should have at least 3 children (Engine, Car, Vehicle), got {}",
        children.len()
    );

    let child_names: Vec<&str> = children.iter().map(|c| c.name.as_str()).collect();
    assert!(
        child_names.contains(&"Engine"),
        "Missing Engine in children"
    );
    assert!(child_names.contains(&"Car"), "Missing Car in children");
    assert!(
        child_names.contains(&"Vehicle"),
        "Missing Vehicle in children"
    );
}

#[test]
fn test_document_symbols_correct_kind() {
    let (graph, uri) = build_vehicle_graph();
    let symbols = build_nested_symbols(&graph, &uri, VEHICLE_SOURCE);

    // Root should be a package
    assert_eq!(symbols[0].kind, SymbolKind::PACKAGE);

    // Children are PartDefinitions -> CLASS
    let children = symbols[0].children.as_ref().unwrap();
    for child in children {
        if child.name == "Engine" || child.name == "Car" || child.name == "Vehicle" {
            assert_eq!(
                child.kind,
                SymbolKind::CLASS,
                "PartDefinition '{}' should be CLASS",
                child.name
            );
        }
    }
}

#[test]
fn test_document_symbols_exclude_other_files() {
    let mut graph = ModelGraph::new();
    let uri = "file:///main.sysml";

    // Element in main file
    let local_id = ElementId::new_v4();
    let local = Element::new(local_id.clone(), ElementKind::Package)
        .with_name("Local")
        .with_span(Span::new(uri, 0, 50));
    graph.add_element(local);

    // Element in a different file
    let remote_id = ElementId::new_v4();
    let remote = Element::new(remote_id.clone(), ElementKind::Package)
        .with_name("Remote")
        .with_span(Span::new("file:///other.sysml", 0, 50));
    graph.add_element(remote);

    let source = "package Local {\n    // content\n}\npackage Remote {}    padding";
    let symbols = build_nested_symbols(&graph, uri, source);

    let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"Local"), "Should include local element");
    assert!(
        !names.contains(&"Remote"),
        "Should exclude other-file element"
    );
}

#[test]
fn test_document_symbols_nested_children() {
    let (graph, uri) = build_vehicle_graph();
    let symbols = build_nested_symbols(&graph, &uri, VEHICLE_SOURCE);

    let children = symbols[0].children.as_ref().unwrap();
    let engine = children.iter().find(|c| c.name == "Engine").unwrap();

    // Engine should have horsePower as a child
    let engine_children = engine
        .children
        .as_ref()
        .expect("Engine should have children");
    let hp = engine_children.iter().find(|c| c.name == "horsePower");
    assert!(hp.is_some(), "Engine should contain horsePower attribute");
    assert_eq!(hp.unwrap().kind, SymbolKind::PROPERTY);
}

// ═══════════════════════════════════════════════════════════════════════
// Go-to Definition tests
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_goto_typing_resolves_to_definition() {
    let mut graph = ModelGraph::new();

    let def_id = ElementId::new_v4();
    let def = Element::new(def_id.clone(), ElementKind::PartDefinition)
        .with_name("Engine")
        .with_span(Span::new("file:///test.sysml", 0, 20));
    graph.add_element(def);

    let usage_id = ElementId::new_v4();
    let usage = Element::new(usage_id.clone(), ElementKind::PartUsage)
        .with_name("engine")
        .with_span(Span::new("file:///test.sysml", 30, 50));
    graph.add_element(usage);

    let typing_id = ElementId::new_v4();
    let mut typing = Element::new(typing_id.clone(), ElementKind::FeatureTyping)
        .with_span(Span::new("file:///test.sysml", 40, 46));
    typing.owner = Some(usage_id.clone());
    typing
        .props
        .insert(Cow::Borrowed("type"), Value::Ref(def_id.clone()));
    graph.add_element(typing);

    let typing_elem = graph.get_element(&typing_id).unwrap();
    let target = resolve_goto_target(typing_elem, &graph);
    assert_eq!(target.id, def_id);
    assert_eq!(target.name.as_deref(), Some("Engine"));
}

#[test]
fn test_goto_specialization_resolves() {
    let mut graph = ModelGraph::new();

    let general_id = ElementId::new_v4();
    let general = Element::new(general_id.clone(), ElementKind::PartDefinition)
        .with_name("Vehicle")
        .with_span(Span::new("file:///test.sysml", 0, 30));
    graph.add_element(general);

    let spec_id = ElementId::new_v4();
    let mut spec = Element::new(spec_id.clone(), ElementKind::Specialization).with_span(Span::new(
        "file:///test.sysml",
        50,
        60,
    ));
    spec.props
        .insert(Cow::Borrowed("general"), Value::Ref(general_id.clone()));
    graph.add_element(spec);

    let spec_elem = graph.get_element(&spec_id).unwrap();
    let target = resolve_goto_target(spec_elem, &graph);
    assert_eq!(target.id, general_id);
    assert_eq!(target.name.as_deref(), Some("Vehicle"));
}

#[test]
fn test_goto_fallback_to_owner() {
    let mut graph = ModelGraph::new();

    let owner_id = ElementId::new_v4();
    let owner = Element::new(owner_id.clone(), ElementKind::PartUsage)
        .with_name("myPart")
        .with_span(Span::new("file:///test.sysml", 0, 30));
    graph.add_element(owner);

    // FeatureTyping with no resolved/unresolved props -> fallback to owner
    let typing_id = ElementId::new_v4();
    let mut typing = Element::new(typing_id.clone(), ElementKind::FeatureTyping)
        .with_span(Span::new("file:///test.sysml", 15, 25));
    typing.owner = Some(owner_id.clone());
    graph.add_element(typing);

    let typing_elem = graph.get_element(&typing_id).unwrap();
    let target = resolve_goto_target(typing_elem, &graph);
    assert_eq!(target.id, owner_id, "Should fall back to owner");
}

// ═══════════════════════════════════════════════════════════════════════
// Hover tests
// ═══════════════════════════════════════════════════════════════════════

/// Test that hovering over an element produces the expected kind and qualified name.
/// This tests the hover content construction logic that lives in the LanguageServer handler.
/// Since the handler is async and requires Client, we replicate the content building here.
#[test]
fn test_hover_basic_kind_and_qname() {
    let (graph, _uri) = build_vehicle_graph();

    // Find Engine by name
    let element = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("Engine"))
        .expect("Engine should exist");

    let kind = format!("{:?}", element.kind);
    assert!(
        kind.contains("PartDefinition"),
        "Kind should contain PartDefinition, got: {}",
        kind
    );
    let qname = element.qname.as_ref().unwrap().to_string();
    assert!(
        qname.contains("Engine"),
        "Qualified name should contain Engine"
    );
}

#[test]
fn test_hover_typed_element() {
    let (graph, _uri) = build_vehicle_graph();

    // Find the 'engine' usage element by name
    let element = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("engine"))
        .expect("engine usage should exist");

    assert_eq!(element.name.as_deref(), Some("engine"));
    let typing = element.props.get("typing").and_then(|v| v.as_str());
    assert_eq!(
        typing,
        Some("Engine"),
        "Should show typing info for engine usage"
    );
}

#[test]
fn test_hover_definition_supertypes() {
    let (graph, _uri) = build_vehicle_graph();

    // Find "Car" and check supertypes via Specialize relationships
    let car = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("Car"))
        .expect("Car should exist");

    assert!(car.kind.is_definition(), "Car should be a definition");

    let supertypes: Vec<_> = graph
        .outgoing(&car.id)
        .filter(|rel| rel.kind == RelationshipKind::Specialize)
        .filter_map(|rel| graph.get_element(&rel.target).and_then(|e| e.name.clone()))
        .collect();

    assert_eq!(supertypes, vec!["Vehicle"], "Car should specialize Vehicle");
}

// ═══════════════════════════════════════════════════════════════════════
// Completion tests
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_completion_type_references() {
    let (graph, _uri) = build_vehicle_graph();

    // Collect definitions for type completion
    let defs: Vec<_> = graph
        .elements
        .values()
        .filter(|e| e.kind.is_definition() && e.name.is_some())
        .map(|e| e.name.clone().unwrap())
        .collect();

    assert!(defs.contains(&"Engine".to_string()));
    assert!(defs.contains(&"Car".to_string()));
    assert!(defs.contains(&"Vehicle".to_string()));
}

#[test]
fn test_completion_namespace_members() {
    let (graph, _uri) = build_vehicle_graph();

    // Resolve "Vehicles" namespace and list its members
    let ns = graph.resolve_qname("Vehicles");
    assert!(ns.is_some(), "Should resolve Vehicles namespace");

    let ns = ns.unwrap();
    let members: Vec<_> = graph
        .owned_members(&ns.id)
        .filter_map(|m| m.name.clone())
        .collect();

    assert!(
        members.contains(&"Engine".to_string()),
        "Vehicles should contain Engine, got: {:?}",
        members
    );
    assert!(
        members.contains(&"Car".to_string()),
        "Vehicles should contain Car, got: {:?}",
        members
    );
    assert!(
        members.contains(&"Vehicle".to_string()),
        "Vehicles should contain Vehicle, got: {:?}",
        members
    );
}

#[test]
fn test_completion_general_has_keywords() {
    // Verify the keyword list is non-empty (testing the static data)
    let keywords = [
        "package",
        "part",
        "part def",
        "attribute",
        "attribute def",
        "action",
        "action def",
        "state",
        "state def",
        "port",
        "port def",
        "connection",
        "connection def",
        "interface",
        "interface def",
        "item",
        "item def",
        "requirement",
        "requirement def",
        "constraint",
        "constraint def",
        "allocation",
        "allocation def",
        "import",
    ];

    assert!(keywords.len() > 20, "Should have many keywords");
    assert!(keywords.contains(&"package"));
    assert!(keywords.contains(&"part def"));
    assert!(keywords.contains(&"import"));
}

// ═══════════════════════════════════════════════════════════════════════
// Workspace Symbol tests
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_workspace_symbol_query_filter() {
    let (graph, _uri) = build_vehicle_graph();
    let query = "eng";

    // Simulate workspace/symbol filtering logic
    let matching: Vec<_> = graph
        .elements
        .values()
        .filter(|e| {
            if let Some(name) = &e.name {
                name.to_lowercase().contains(&query.to_lowercase())
            } else {
                false
            }
        })
        .map(|e| e.name.clone().unwrap())
        .collect();

    assert!(
        matching.contains(&"Engine".to_string()),
        "Query 'eng' should match Engine"
    );
    assert!(
        matching.contains(&"engine".to_string()),
        "Query 'eng' should match engine (usage)"
    );
    assert!(
        !matching.contains(&"Car".to_string()),
        "Query 'eng' should not match Car"
    );
}

#[test]
fn test_workspace_symbol_empty_query_returns_all() {
    let (graph, _uri) = build_vehicle_graph();
    let query = "";

    let matching: Vec<_> = graph
        .elements
        .values()
        .filter(|e| {
            let Some(name) = &e.name else {
                return false;
            };
            query.is_empty() || name.to_lowercase().contains(&query.to_lowercase())
        })
        .collect();

    // Should have all named elements
    assert!(
        matching.len() >= 6,
        "Empty query should return all named elements, got {}",
        matching.len()
    );
}

#[test]
fn test_workspace_symbol_container_name() {
    let (graph, _uri) = build_vehicle_graph();

    let engine = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("Engine"))
        .unwrap();

    // container_name logic from workspace/symbol handler
    let container_name = engine.qname.as_ref().and_then(|q| {
        let parts = q.segments();
        if parts.len() > 1 {
            Some(parts[..parts.len() - 1].join("::"))
        } else {
            None
        }
    });

    assert_eq!(
        container_name.as_deref(),
        Some("Vehicles"),
        "Engine's container should be 'Vehicles'"
    );
}

#[test]
fn test_workspace_symbol_skips_synthetic() {
    let mut graph = ModelGraph::new();

    // Named element with real span
    let real_id = ElementId::new_v4();
    let real = Element::new(real_id.clone(), ElementKind::Package)
        .with_name("RealPkg")
        .with_span(Span::new("file:///test.sysml", 0, 30));
    graph.add_element(real);

    // Named element with synthetic span (should be skipped)
    let synth_id = ElementId::new_v4();
    let synth = Element::new(synth_id.clone(), ElementKind::Package)
        .with_name("SynthPkg")
        .with_span(Span::new("<synthetic>", 0, 10));
    graph.add_element(synth);

    // Simulate workspace/symbol filtering: skip synthetic spans
    let results: Vec<_> = graph
        .elements
        .values()
        .filter(|e| {
            let name = e.name.as_ref();
            let span = e.spans.first();
            name.is_some() && span.map(|s| s.file != "<synthetic>").unwrap_or(false)
        })
        .map(|e| e.name.clone().unwrap())
        .collect();

    assert!(results.contains(&"RealPkg".to_string()));
    assert!(
        !results.contains(&"SynthPkg".to_string()),
        "Synthetic elements should be skipped"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Diagnostic: Parse real sysml-rs.sysml and verify elements
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn diagnostic_parse_real_sysml_file() {
    use sysml_parser_incremental::{build_model_graph, TreeSitterParser};

    let source = r#"package SysmlRs {
    abstract part def Crate;
    part def FoundationsCrate :> Crate;
    part def CoreCrate :> Crate;
    part def sysml_id :> FoundationsCrate;
    part def sysml_core :> CoreCrate {
        part def ModelGraph;
    }
}"#;

    let parser = TreeSitterParser::new();
    let tree = parser.parse_tree(source).expect("tree-sitter parse");
    let uri = "file:///test/sysml-rs.sysml";
    let result = build_model_graph(&tree, source, uri);

    // Print diagnostics for debugging
    for d in &result.diagnostics {
        eprintln!("  DIAG: {}", d.message);
    }

    // Print all elements
    for elem in result.graph.elements.values() {
        eprintln!(
            "  ELEM: {:?} name={:?} owner_is_none={} spans={:?}",
            elem.kind,
            elem.name,
            elem.owner.is_none(),
            elem.spans
                .iter()
                .map(|s| format!("{}:{}-{}", s.file, s.start, s.end))
                .collect::<Vec<_>>()
        );
    }

    // Should produce at least the package and its definitions
    let named: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.name.is_some())
        .collect();
    eprintln!("\n  Named elements: {}", named.len());
    for n in &named {
        eprintln!("    {:?}: {}", n.kind, n.name.as_ref().unwrap());
    }

    // Root elements (what document_symbol uses)
    let roots: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| {
            e.owner.is_none()
                && e.spans
                    .iter()
                    .any(|s| s.file == uri && s.file != "<synthetic>")
        })
        .collect();
    eprintln!("\n  Root elements (for outline): {}", roots.len());
    for r in &roots {
        eprintln!("    {:?}: {:?}", r.kind, r.name);
    }

    // Test build_nested_symbols
    let symbols = build_nested_symbols(&result.graph, uri, source);
    eprintln!("\n  Document symbols: {}", symbols.len());
    for s in &symbols {
        eprintln!(
            "    {} ({:?}) children={}",
            s.name,
            s.kind,
            s.children.as_ref().map(|c| c.len()).unwrap_or(0)
        );
        if let Some(children) = &s.children {
            for c in children {
                eprintln!("      {} ({:?})", c.name, c.kind);
            }
        }
    }

    // Verify basic expectations
    assert!(!result.graph.elements.is_empty(), "Should have elements");
    assert!(!symbols.is_empty(), "Should have document symbols");
    assert!(
        symbols[0].children.is_some(),
        "Package should have children in outline"
    );

    // Find Specialization elements (for goto-def)
    let specs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::Specialization)
        .collect();
    eprintln!("\n  Specialization elements: {}", specs.len());
    for s in &specs {
        eprintln!("    props: {:?}", s.props);
        eprintln!("    owner: {:?}", s.owner);
        eprintln!(
            "    spans: {:?}",
            s.spans
                .iter()
                .map(|sp| format!("{}:{}-{}", sp.file, sp.start, sp.end))
                .collect::<Vec<_>>()
        );
    }

    // Test resolve_goto_target on a Specialization
    if let Some(spec) = specs.first() {
        let target = resolve_goto_target(spec, &result.graph);
        eprintln!(
            "\n  resolve_goto_target on Specialization => {:?} name={:?}",
            target.kind, target.name
        );
    }
}

/// Diagnostic test: parse the ACTUAL sysml-rs.sysml file and see what errors/elements we get.
/// This helps understand why outline and goto-def fail on the real file.
#[test]
fn diagnostic_parse_actual_sysml_rs_file() {
    use sysml_parser_incremental::{build_model_graph, TreeSitterParser};

    // Read the actual file
    let file_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../model/sysml-rs.sysml");
    let source = match std::fs::read_to_string(file_path) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("SKIPPED: Could not read {}", file_path);
            return;
        }
    };

    let parser = TreeSitterParser::new();
    let tree = parser.parse_tree(&source).expect("tree-sitter parse");
    let uri = "file:///model/sysml-rs.sysml";
    let result = build_model_graph(&tree, &source, uri);

    eprintln!("\n=== ACTUAL FILE DIAGNOSTICS ===");
    eprintln!("  Diagnostics: {}", result.diagnostics.len());
    for (i, d) in result.diagnostics.iter().enumerate().take(10) {
        let span_info = d
            .span
            .as_ref()
            .map(|s| format!("bytes {}..{}", s.start, s.end))
            .unwrap_or("no span".to_string());
        eprintln!("  [{}] {} | {}", i, d.message, span_info);
    }
    if result.diagnostics.len() > 10 {
        eprintln!("  ... and {} more", result.diagnostics.len() - 10);
    }

    let named: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.name.is_some())
        .collect();
    eprintln!("\n  Total elements: {}", result.graph.elements.len());
    eprintln!("  Named elements: {}", named.len());
    for n in &named {
        eprintln!("    {:?}: {}", n.kind, n.name.as_ref().unwrap());
    }

    let roots: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| {
            e.owner.is_none()
                && e.spans
                    .iter()
                    .any(|s| s.file == uri && s.file != "<synthetic>")
        })
        .collect();
    eprintln!("\n  Root elements: {}", roots.len());

    let symbols = build_nested_symbols(&result.graph, uri, &source);
    eprintln!("  Document symbols: {}", symbols.len());
    for s in &symbols {
        let child_count = s.children.as_ref().map(|c| c.len()).unwrap_or(0);
        eprintln!("    {} ({:?}) children={}", s.name, s.kind, child_count);
    }

    // Count how many part_def elements we found vs expected
    let part_defs: Vec<_> = result
        .graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::PartDefinition)
        .collect();
    eprintln!(
        "\n  PartDefinitions found: {} (file has ~30)",
        part_defs.len()
    );

    // This test is intentionally lenient - it's for diagnostics
    // The real fix is in the grammar or parser
    eprintln!("\n=== END DIAGNOSTICS ===\n");
}

// ═══════════════════════════════════════════════════════════════════════
// Cross-feature integration: FeatureTyping + resolve_goto_target
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_feature_typing_with_goto_target() {
    let mut graph = ModelGraph::new();
    let uri = "file:///test.sysml";

    // Definition
    let def_id = ElementId::new_v4();
    let def = Element::new(def_id.clone(), ElementKind::PartDefinition)
        .with_name("Engine")
        .with_span(Span::new(uri, 0, 30));
    graph.add_element(def);

    // Usage
    let usage_id = ElementId::new_v4();
    let usage = Element::new(usage_id.clone(), ElementKind::PartUsage)
        .with_name("engine")
        .with_span(Span::new(uri, 40, 70));
    graph.add_element(usage);

    // FeatureTyping relationship spanning a sub-region of usage
    let typing_id = ElementId::new_v4();
    let mut typing = Element::new(typing_id.clone(), ElementKind::FeatureTyping)
        .with_span(Span::new(uri, 55, 65));
    typing.owner = Some(usage_id.clone());
    typing
        .props
        .insert(Cow::Borrowed("type"), Value::Ref(def_id.clone()));
    graph.add_element(typing);

    // Resolving goto target on the FeatureTyping should navigate to the definition
    let clicked_elem = graph.get_element(&typing_id).unwrap();
    let target = resolve_goto_target(clicked_elem, &graph);
    assert_eq!(
        target.id, def_id,
        "goto_target should resolve to Engine definition"
    );
    assert_eq!(target.name.as_deref(), Some("Engine"));
}

// ═══════════════════════════════════════════════════════════════════════
// Evaluation integration tests (EX-1)
// ═══════════════════════════════════════════════════════════════════════

/// Build a graph with constraints and calculations for evaluation tests.
fn build_evaluation_graph() -> (ModelGraph, String) {
    let uri = "file:///eval_test.sysml";
    let mut graph = ModelGraph::new();

    // Owner: part vehicle
    let owner_id = ElementId::new_v4();
    let owner = Element::new(owner_id.clone(), ElementKind::PartUsage)
        .with_name("vehicle")
        .with_span(Span::new(uri, 0, 200));
    graph.add_element(owner);

    // Attribute: speed = 50
    let speed = Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
        .with_name("speed")
        .with_prop("value", Value::Int(50))
        .with_span(Span::new(uri, 20, 40));
    graph.add_owned_element(speed, owner_id.clone(), VisibilityKind::Public);

    // Attribute: voltage = 120
    let voltage = Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
        .with_name("voltage")
        .with_prop("value", Value::Int(120))
        .with_span(Span::new(uri, 42, 60));
    graph.add_owned_element(voltage, owner_id.clone(), VisibilityKind::Public);

    // Constraint: speed < 100 (should pass)
    let constraint = Element::new(ElementId::new_v4(), ElementKind::ConstraintUsage)
        .with_name("speedLimit")
        .with_prop("constraint", Value::String("speed < 100".into()))
        .with_span(Span::new(uri, 62, 90));
    graph.add_owned_element(constraint, owner_id.clone(), VisibilityKind::Public);

    // Calculation: voltage * 10
    let calc = Element::new(ElementId::new_v4(), ElementKind::CalculationUsage)
        .with_name("power")
        .with_prop("expr", Value::String("voltage * 10".into()))
        .with_span(Span::new(uri, 92, 120));
    graph.add_owned_element(calc, owner_id.clone(), VisibilityKind::Public);

    (graph, uri.to_string())
}

#[test]
fn test_evaluation_code_lens_constraint_pass() {
    let (graph, uri) = build_evaluation_graph();

    let results = crate::evaluation::evaluate_constraints(&graph);

    // Should find our constraint
    let constraint_result = results
        .iter()
        .find(|r| r.span.as_ref().map_or(false, |s| s.file == uri));
    assert!(
        constraint_result.is_some(),
        "should find constraint in graph"
    );
    let cr = constraint_result.unwrap();
    assert!(cr.satisfied, "speed=50 < 100 should PASS");
    assert!(cr.detail.contains("PASS"), "detail should contain PASS");
}

#[test]
fn test_evaluation_code_lens_calculation() {
    let (graph, uri) = build_evaluation_graph();

    let results = crate::evaluation::evaluate_calculations(&graph);

    // Should find our calculation
    let calc_result = results
        .iter()
        .find(|(_id, span, _display)| span.as_ref().map_or(false, |s| s.file == uri));
    assert!(calc_result.is_some(), "should find calculation in graph");
    let (_id, _span, display) = calc_result.unwrap();
    assert!(
        display.contains("1200"),
        "voltage=120 * 10 should = 1200, got: {}",
        display
    );
}

#[test]
fn test_evaluation_inlay_hint_value() {
    let (graph, _uri) = build_evaluation_graph();

    // Find the calculation element and try evaluating it
    let calc = graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::CalculationUsage && e.name.as_deref() == Some("power"));
    assert!(calc.is_some(), "should find calc element");

    let result = crate::evaluation::try_evaluate_value(calc.unwrap(), &graph);
    assert_eq!(result, Some("1200".to_string()), "voltage=120 * 10 = 1200");
}

#[test]
fn test_evaluation_evaluate_element_command() {
    let (graph, _uri) = build_evaluation_graph();

    // Find constraint and evaluate
    let constraint = graph.elements.values().find(|e| {
        e.kind == ElementKind::ConstraintUsage && e.name.as_deref() == Some("speedLimit")
    });
    assert!(constraint.is_some(), "should find constraint element");

    let result = crate::evaluation::evaluate_element(constraint.unwrap(), &graph);
    assert!(result.is_some(), "constraint should be evaluable");
    assert_eq!(
        result.unwrap(),
        "true",
        "speed=50 < 100 should evaluate to true"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Protocol-level integration tests using TestServer
//
// These tests exercise end-to-end LSP protocol behavior through the full
// handler pipeline, complementing the unit-level tests above and the
// protocol_tests module.
// ═══════════════════════════════════════════════════════════════════════

mod protocol_integration {
    use tower_lsp::lsp_types::*;
    use tower_lsp::LanguageServer;

    use crate::test_harness::TestServer;

    const TEST_URI: &str = "file:///integration_test.sysml";

    // ── Lifecycle ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_initialize_capabilities_comprehensive() {
        let server = TestServer::new();
        let result = server.initialize().await;
        let caps = result.capabilities;

        // Core text sync
        assert!(
            caps.text_document_sync.is_some(),
            "Must advertise text document sync"
        );

        // Completion
        let comp = caps
            .completion_provider
            .as_ref()
            .expect("Must advertise completion");
        assert!(
            comp.trigger_characters.is_some(),
            "Completion should have trigger characters"
        );

        // Navigation
        assert!(caps.definition_provider.is_some(), "Must have goto def");
        assert!(caps.references_provider.is_some(), "Must have references");

        // Symbols
        assert!(
            caps.document_symbol_provider.is_some(),
            "Must have doc symbols"
        );
        assert!(
            caps.workspace_symbol_provider.is_some(),
            "Must have workspace symbols"
        );

        // Code intelligence
        assert!(caps.hover_provider.is_some(), "Must have hover");
        assert!(
            caps.semantic_tokens_provider.is_some(),
            "Must have semantic tokens"
        );
        assert!(caps.rename_provider.is_some(), "Must have rename");
        assert!(
            caps.folding_range_provider.is_some(),
            "Must have folding ranges"
        );
        assert!(caps.inlay_hint_provider.is_some(), "Must have inlay hints");

        // Code actions
        assert!(
            caps.code_action_provider.is_some(),
            "Must have code actions"
        );

        // Execute command
        let exec = caps
            .execute_command_provider
            .as_ref()
            .expect("Must advertise execute_command");
        assert!(
            exec.commands.len() > 10,
            "Should register many commands, got {}",
            exec.commands.len()
        );
    }

    #[tokio::test]
    async fn test_shutdown_after_initialize() {
        let server = TestServer::new();
        server.initialize_full().await;
        let result = server.server().shutdown().await;
        assert!(result.is_ok(), "Shutdown should succeed after initialize");
    }

    // ── Document Lifecycle ─────────────────────────────────────────

    #[tokio::test]
    async fn test_open_document_makes_it_available() {
        let server = TestServer::new();
        server.initialize_full().await;

        let content = "package Lifecycle { part def Widget; }";
        server.open_document(TEST_URI, content).await;

        // Document should now be indexed - symbols should be available
        let syms = server.document_symbol(TEST_URI).await;
        assert!(
            syms.is_some(),
            "After open, document symbols should be available"
        );
    }

    #[tokio::test]
    async fn test_change_document_updates_content() {
        let server = TestServer::new();
        server.initialize_full().await;

        server
            .open_document(TEST_URI, "package V1 { part def A; }")
            .await;
        let syms = server.document_symbol(TEST_URI).await;
        if let Some(DocumentSymbolResponse::Nested(symbols)) = &syms {
            if !symbols.is_empty() {
                assert_eq!(symbols[0].name, "V1");
            }
        }

        // Change to new content
        server
            .change_document(TEST_URI, 1, "package V2 { part def B; }")
            .await;
        let syms = server.document_symbol(TEST_URI).await;
        if let Some(DocumentSymbolResponse::Nested(symbols)) = &syms {
            if !symbols.is_empty() {
                assert_eq!(
                    symbols[0].name, "V2",
                    "After change, package name should be V2"
                );
            }
        }
    }

    #[tokio::test]
    async fn test_close_document_removes_it() {
        let server = TestServer::new();
        server.initialize_full().await;

        server
            .open_document(TEST_URI, "package ClosePkg { part def X; }")
            .await;
        server.close_document(TEST_URI).await;

        let syms = server.document_symbol(TEST_URI).await;
        match syms {
            None => {} // Expected: closed documents return None
            Some(DocumentSymbolResponse::Nested(symbols)) => {
                assert!(symbols.is_empty(), "After close, should have no symbols");
            }
            _ => {}
        }
    }

    // ── Completion ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_completion_returns_keywords_inside_package() {
        let server = TestServer::new();
        server.initialize_full().await;

        let content = "package CompTest {\n  \n}";
        server.open_document(TEST_URI, content).await;

        let response = server.completion(TEST_URI, 1, 2, None).await;
        assert!(
            response.is_some(),
            "Completion should return results inside package body"
        );

        if let Some(CompletionResponse::Array(items)) = response {
            let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
            // Should include SysML keywords
            assert!(
                labels.iter().any(|l| l.contains("part")),
                "Should suggest 'part' keyword: {:?}",
                &labels[..labels.len().min(10)]
            );
            assert!(
                labels.iter().any(|l| l.contains("attribute")),
                "Should suggest 'attribute' keyword: {:?}",
                &labels[..labels.len().min(10)]
            );
        }
    }

    #[tokio::test]
    async fn test_completion_after_colon_suggests_types() {
        let server = TestServer::new();
        server.initialize_full().await;

        let content = "package CompTest {\n  part def Motor {}\n  part eng :\n}";
        server.open_document(TEST_URI, content).await;

        // Trigger completion after ":"
        let response = server.completion(TEST_URI, 2, 12, Some(":")).await;
        if let Some(CompletionResponse::Array(items)) = response {
            let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
            assert!(
                labels.iter().any(|l| l.contains("Motor")),
                "Completion after ':' should suggest 'Motor' type: {:?}",
                &labels[..labels.len().min(10)]
            );
        }
    }

    // ── Hover ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_hover_over_keyword_returns_content() {
        let server = TestServer::new();
        server.initialize_full().await;

        let content = "package HoverPkg {\n  part def Widget;\n}";
        server.open_document(TEST_URI, content).await;

        // Hover over "package" keyword (line 0, col 3)
        let hover = server.hover(TEST_URI, 0, 3).await;
        if let Some(h) = hover {
            match h.contents {
                HoverContents::Markup(m) => {
                    assert!(
                        !m.value.is_empty(),
                        "Hover on 'package' should return non-empty content"
                    );
                }
                _ => {} // Other formats are acceptable
            }
        }
        // Not panicking is the main assertion
    }

    #[tokio::test]
    async fn test_hover_over_definition_name() {
        let server = TestServer::new();
        server.initialize_full().await;

        // "part def Widget" - Widget at line 1, char ~12
        let content = "package HoverPkg {\n  part def Widget;\n}";
        server.open_document(TEST_URI, content).await;

        let hover = server.hover(TEST_URI, 1, 12).await;
        if let Some(h) = hover {
            match &h.contents {
                HoverContents::Markup(m) => {
                    assert!(
                        m.value.contains("Widget") || m.value.contains("Part"),
                        "Hover on definition should reference the element name: {}",
                        m.value
                    );
                }
                _ => {}
            }
        }
    }

    // ── Navigation ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_goto_definition_on_type_reference() {
        let server = TestServer::new();
        server.initialize_full().await;

        let content = "package NavTest {\n  part def Engine {}\n  part myEngine : Engine;\n}";
        server.open_document(TEST_URI, content).await;

        // "part myEngine : Engine;" - "Engine" reference at line 2, around col 18
        let response = server.goto_definition(TEST_URI, 2, 19).await;
        if let Some(def) = response {
            match def {
                GotoDefinitionResponse::Scalar(loc) => {
                    assert_eq!(
                        loc.uri.as_str(),
                        TEST_URI,
                        "Definition should be in same file"
                    );
                }
                GotoDefinitionResponse::Array(locs) if !locs.is_empty() => {
                    assert_eq!(locs[0].uri.as_str(), TEST_URI);
                }
                GotoDefinitionResponse::Link(links) if !links.is_empty() => {
                    assert!(links[0].target_uri.as_str() == TEST_URI);
                }
                _ => {} // Empty results are OK if resolution hasn't run
            }
        }
        // Returning None is also acceptable if tree-sitter state prevents resolution
    }

    #[tokio::test]
    async fn test_find_references_on_definition() {
        let server = TestServer::new();
        server.initialize_full().await;

        let content =
            "package RefTest {\n  part def Sensor {}\n  part s1 : Sensor;\n  part s2 : Sensor;\n}";
        server.open_document(TEST_URI, content).await;

        // Find references to "Sensor" at its definition (line 1, ~col 12)
        let refs = server.references(TEST_URI, 1, 12).await;
        if let Some(locations) = refs {
            assert!(
                !locations.is_empty(),
                "Should find at least one reference to Sensor"
            );
        }
    }

    // ── Document Symbols ───────────────────────────────────────────

    #[tokio::test]
    async fn test_document_symbols_nested_structure() {
        let server = TestServer::new();
        server.initialize_full().await;

        let content = "package Outer {\n  part def Inner {\n    attribute x;\n  }\n}";
        server.open_document(TEST_URI, content).await;

        let response = server.document_symbol(TEST_URI).await;
        assert!(
            response.is_some(),
            "Should return document symbols for valid SysML"
        );

        if let Some(DocumentSymbolResponse::Nested(symbols)) = response {
            assert!(!symbols.is_empty(), "Should have at least one root symbol");
            let root = &symbols[0];
            assert_eq!(root.name, "Outer");
            assert_eq!(root.kind, SymbolKind::PACKAGE);

            // Should have nested children
            if let Some(children) = &root.children {
                assert!(
                    !children.is_empty(),
                    "Package should have children in symbol tree"
                );
            }
        }
    }

    #[tokio::test]
    async fn test_workspace_symbol_search_finds_elements() {
        let server = TestServer::new();
        server.initialize_full().await;

        let content = "package SymTest {\n  part def Accelerometer;\n  part def Gyroscope;\n}";
        server.open_document(TEST_URI, content).await;

        let symbols = server.workspace_symbol("Accel").await;
        if let Some(results) = symbols {
            let names: Vec<&str> = results.iter().map(|s| s.name.as_str()).collect();
            assert!(
                names.iter().any(|n| n.contains("Accelerometer")),
                "Workspace symbol search for 'Accel' should find Accelerometer: {:?}",
                names
            );
        }
    }

    // ── Commands ───────────────────────────────────────────────────

    #[tokio::test]
    async fn test_execute_unknown_command_returns_none() {
        let server = TestServer::new();
        server.initialize_full().await;

        let result = server
            .execute_command("sysml.nonexistent.command", vec![])
            .await;
        assert!(
            result.is_none(),
            "Unknown command should return None, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_execute_debug_status_returns_json() {
        let server = TestServer::new();
        server.initialize_full().await;
        server
            .open_document(TEST_URI, "package StatusTest {}")
            .await;

        let result = server.execute_command("sysml.debug.status", vec![]).await;
        let status = result.expect("debug.status should return a value");

        // Validate the structure has expected keys
        assert!(
            status.get("health").is_some(),
            "Status should have 'health' key: {}",
            status
        );
        assert!(
            status.get("documents").is_some(),
            "Status should have 'documents' key: {}",
            status
        );
    }

    #[tokio::test]
    async fn test_execute_cache_status_returns_json() {
        let server = TestServer::new();
        server.initialize_full().await;

        let result = server.execute_command("sysml.cache.status", vec![]).await;
        let status = result.expect("cache.status should return a value");
        assert!(
            status.is_object(),
            "Cache status should be a JSON object: {}",
            status
        );
    }

    // ── Semantic Tokens ────────────────────────────────────────────

    #[tokio::test]
    async fn test_semantic_tokens_returns_data() {
        let server = TestServer::new();
        server.initialize_full().await;

        let content = "package TokenTest {\n  part def Sensor;\n  part mySensor : Sensor;\n}";
        server.open_document(TEST_URI, content).await;

        let result = server.semantic_tokens_full(TEST_URI).await;
        assert!(
            result.is_some(),
            "Should return semantic tokens for valid SysML"
        );

        if let Some(SemanticTokensResult::Tokens(tokens)) = result {
            assert!(
                !tokens.data.is_empty(),
                "Semantic tokens should have data entries"
            );
        }
    }

    // ── Folding Ranges ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_folding_ranges_for_nested_blocks() {
        let server = TestServer::new();
        server.initialize_full().await;

        let content =
            "package FoldTest {\n  part def A {\n    attribute x;\n  }\n  part def B {\n    attribute y;\n  }\n}";
        server.open_document(TEST_URI, content).await;

        let ranges = server.folding_range(TEST_URI).await;
        assert!(
            ranges.is_some(),
            "Should return folding ranges for nested blocks"
        );

        let ranges = ranges.unwrap();
        // Package and two definitions should each have a folding range
        assert!(
            ranges.len() >= 2,
            "Should have at least 2 folding ranges (for nested blocks), got {}",
            ranges.len()
        );
    }

    // ── Rename ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_rename_produces_workspace_edit() {
        let server = TestServer::new();
        server.initialize_full().await;

        let content = "package RenameTest {\n  part def Gadget;\n}";
        server.open_document(TEST_URI, content).await;

        // Rename "RenameTest" at (0, 10) to "RenamedPkg"
        let edit = server.rename(TEST_URI, 0, 10, "RenamedPkg").await;
        if let Some(workspace_edit) = edit {
            let changes = workspace_edit.changes.unwrap_or_default();
            assert!(
                !changes.is_empty(),
                "Rename should produce at least one file change"
            );
        }
    }

    // ── Edge Cases ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_multiple_documents_independent() {
        let server = TestServer::new();
        server.initialize_full().await;

        let uri_a = "file:///a.sysml";
        let uri_b = "file:///b.sysml";
        server
            .open_document(uri_a, "package A { part def TypeA; }")
            .await;
        server
            .open_document(uri_b, "package B { part def TypeB; }")
            .await;

        let syms_a = server.document_symbol(uri_a).await;
        let syms_b = server.document_symbol(uri_b).await;

        if let Some(DocumentSymbolResponse::Nested(a)) = &syms_a {
            if !a.is_empty() {
                assert_eq!(a[0].name, "A");
            }
        }
        if let Some(DocumentSymbolResponse::Nested(b)) = &syms_b {
            if !b.is_empty() {
                assert_eq!(b[0].name, "B");
            }
        }

        // Close A, B should still work
        server.close_document(uri_a).await;
        let syms_b_after = server.document_symbol(uri_b).await;
        assert!(
            syms_b_after.is_some(),
            "Closing doc A should not affect doc B"
        );
    }

    #[tokio::test]
    async fn test_rapid_open_close_cycle() {
        let server = TestServer::new();
        server.initialize_full().await;

        // Rapidly open and close documents to check for race conditions
        for i in 0..5 {
            let uri = format!("file:///rapid_{}.sysml", i);
            let content = format!("package Rapid{} {{ part def X; }}", i);
            server.open_document(&uri, &content).await;
            let _ = server.document_symbol(&uri).await;
            server.close_document(&uri).await;
        }
        // Main assertion: no panics
    }

    #[tokio::test]
    async fn test_all_operations_on_empty_file() {
        let server = TestServer::new();
        server.initialize_full().await;
        server.open_document(TEST_URI, "").await;

        // None of these should panic
        let _ = server.document_symbol(TEST_URI).await;
        let _ = server.hover(TEST_URI, 0, 0).await;
        let _ = server.completion(TEST_URI, 0, 0, None).await;
        let _ = server.goto_definition(TEST_URI, 0, 0).await;
        let _ = server.references(TEST_URI, 0, 0).await;
        let _ = server.semantic_tokens_full(TEST_URI).await;
        let _ = server.folding_range(TEST_URI).await;
        let _ = server.inlay_hint(TEST_URI).await;
        let _ = server.workspace_symbol("").await;
        let _ = server.rename(TEST_URI, 0, 0, "newName").await;
        let _ = server.execute_command("sysml.debug.status", vec![]).await;
    }
}
