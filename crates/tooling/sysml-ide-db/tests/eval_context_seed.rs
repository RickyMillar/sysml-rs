//! Integration tests for `eval_context_seed::context_from_graph`.
//!
//! These tests construct synthetic `ModelGraph`s directly (no parser, no
//! AnalysisHost) and verify the seed walk: ISQ auto-tagging across the three
//! type-resolution strategies, and the occurrence-registry attachment
//! invariant.
//!
//! Originally lived in `sysml-runtime::compiler` next to the seed walk;
//! moved here with the function itself per ADR-011 §3 (Option B).

use std::sync::Arc;

use sysml_core::physics::DimensionVector;
use sysml_core::{Element, ElementId, ElementKind, ModelGraph, Value};
use sysml_ide_db::eval_context_seed::context_from_graph;

#[test]
fn test_isq_auto_tagging_length_value() {
    let mut graph = ModelGraph::new();

    // Attribute: attribute length : LengthValue = 5.0;
    let attr_id = ElementId::new_v4();
    let mut attr = Element::new(attr_id.clone(), ElementKind::AttributeUsage);
    attr.name = Some("length".to_string());
    attr.set_prop("default", Value::Float(5.0));
    attr.set_prop("unresolvedTypeName", Value::String("LengthValue".to_string()));
    graph.add_element(attr);

    graph.rebuild_indexes();
    let graph = Arc::new(graph);
    let ctx = context_from_graph(&graph);

    let val = ctx.get("length").unwrap();
    match val {
        Value::Quantity { value, dimension, .. } => {
            assert_eq!(*value, 5.0);
            // Length dimension: L^1
            assert_eq!(*dimension, DimensionVector::new(1, 0, 0, 0, 0, 0, 0));
        }
        other => panic!("expected Quantity, got {:?}", other),
    }
}

#[test]
fn test_isq_auto_tagging_with_package_prefix() {
    let mut graph = ModelGraph::new();

    // Attribute with ISQ:: prefix
    let attr_id = ElementId::new_v4();
    let mut attr = Element::new(attr_id.clone(), ElementKind::AttributeUsage);
    attr.name = Some("mass".to_string());
    attr.set_prop("default", Value::Float(10.0));
    attr.set_prop("unresolvedTypeName", Value::String("ISQ::MassValue".to_string()));
    graph.add_element(attr);

    graph.rebuild_indexes();
    let graph = Arc::new(graph);
    let ctx = context_from_graph(&graph);

    let val = ctx.get("mass").unwrap();
    match val {
        Value::Quantity { value, dimension, .. } => {
            assert_eq!(*value, 10.0);
            // Mass dimension: M^1
            assert_eq!(dimension.mass, 1);
        }
        other => panic!("expected Quantity, got {:?}", other),
    }
}

#[test]
fn test_isq_auto_tagging_non_isq_type_unchanged() {
    let mut graph = ModelGraph::new();

    // Attribute with non-ISQ type should remain as plain Float
    let attr_id = ElementId::new_v4();
    let mut attr = Element::new(attr_id.clone(), ElementKind::AttributeUsage);
    attr.name = Some("ratio".to_string());
    attr.set_prop("default", Value::Float(0.5));
    attr.set_prop("unresolvedTypeName", Value::String("Real".to_string()));
    graph.add_element(attr);

    graph.rebuild_indexes();
    let graph = Arc::new(graph);
    let ctx = context_from_graph(&graph);

    let val = ctx.get("ratio").unwrap();
    assert_eq!(val, &Value::Float(0.5), "non-ISQ type should remain as plain Float");
}

#[test]
fn test_isq_auto_tagging_via_feature_typing() {
    let mut graph = ModelGraph::new();

    // Attribute without unresolvedTypeName but with FeatureTyping child
    let attr_id = ElementId::new_v4();
    let mut attr = Element::new(attr_id.clone(), ElementKind::AttributeUsage);
    attr.name = Some("temperature".to_string());
    attr.set_prop("default", Value::Float(300.0));
    graph.add_element(attr);

    // FeatureTyping child with unresolved_type
    let ft_id = ElementId::new_v4();
    let mut ft = Element::new(ft_id.clone(), ElementKind::FeatureTyping);
    ft.owner = Some(attr_id.clone());
    ft.set_prop("unresolved_type", Value::String("ThermodynamicTemperatureValue".to_string()));
    graph.add_element(ft);

    graph.rebuild_indexes();
    let graph = Arc::new(graph);
    let ctx = context_from_graph(&graph);

    let val = ctx.get("temperature").unwrap();
    match val {
        Value::Quantity { value, dimension, .. } => {
            assert_eq!(*value, 300.0);
            // Temperature dimension: Θ^1
            assert_eq!(*dimension, DimensionVector::new(0, 0, 0, 0, 1, 0, 0));
        }
        other => panic!("expected Quantity, got {:?}", other),
    }
}

#[test]
fn test_isq_auto_tagging_no_double_wrap() {
    let mut graph = ModelGraph::new();

    // Attribute already stored as Quantity should not be double-wrapped
    let attr_id = ElementId::new_v4();
    let mut attr = Element::new(attr_id.clone(), ElementKind::AttributeUsage);
    attr.name = Some("force".to_string());
    attr.set_prop(
        "default",
        Value::quantity(
            9.81,
            DimensionVector::new(1, 1, -2, 0, 0, 0, 0),
            Some("N".to_string()),
        ),
    );
    attr.set_prop("unresolvedTypeName", Value::String("ForceValue".to_string()));
    graph.add_element(attr);

    graph.rebuild_indexes();
    let graph = Arc::new(graph);
    let ctx = context_from_graph(&graph);

    let val = ctx.get("force").unwrap();
    match val {
        Value::Quantity { value, unit, .. } => {
            assert_eq!(*value, 9.81);
            assert_eq!(unit.as_deref(), Some("N"), "unit should be preserved");
        }
        other => panic!("expected Quantity, got {:?}", other),
    }
}

#[test]
fn test_context_from_graph_has_occurrence_registry() {
    let graph = Arc::new(ModelGraph::new());
    let ctx = context_from_graph(&graph);
    assert!(ctx.occurrence_registry.is_some());
}
