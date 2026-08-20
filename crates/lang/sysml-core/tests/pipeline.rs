//! End-to-end pipeline tests for elaboration of successions and flows.
//!
//! These tests verify that the elaboration pass correctly derives:
//! - C3: Transition relationships from succession elements in actions
//! - C4: Source/target/payloadType properties from flow endpoint children

use sysml_core::elaborate::elaborate;
use sysml_core::{Element, ElementKind, ModelGraph, RelationshipKind};
use sysml_span::Span;

// ---------------------------------------------------------------------------
// C3: Action succession pipeline
// ---------------------------------------------------------------------------

/// Build an action with succession steps, elaborate, and verify transitions.
#[test]
fn action_succession_elaborate_pipeline() {
    let mut graph = ModelGraph::new();

    // Action definition
    let action = Element::new_with_kind(ElementKind::ActionDefinition).with_name("LaunchSequence");
    let action_id = graph.add_element(action);

    // Steps
    let step1 = Element::new_with_kind(ElementKind::ActionUsage)
        .with_name("ignite")
        .with_owner(action_id.clone());
    let step1_id = graph.add_element(step1);

    let step2 = Element::new_with_kind(ElementKind::ActionUsage)
        .with_name("liftoff")
        .with_owner(action_id.clone());
    let step2_id = graph.add_element(step2);

    let step3 = Element::new_with_kind(ElementKind::ActionUsage)
        .with_name("ascend")
        .with_owner(action_id.clone());
    let step3_id = graph.add_element(step3);

    // Successions: ignite -> liftoff -> ascend
    let s1 = Element::new_with_kind(ElementKind::SuccessionAsUsage)
        .with_owner(action_id.clone())
        .with_prop("source", "ignite")
        .with_prop("target", "liftoff");
    graph.add_element(s1);

    let s2 = Element::new_with_kind(ElementKind::SuccessionAsUsage)
        .with_owner(action_id.clone())
        .with_prop("source", "liftoff")
        .with_prop("target", "ascend");
    graph.add_element(s2);

    // Elaborate
    let report = elaborate(&mut graph);

    assert_eq!(
        report.successions_created, 2,
        "Should create 2 transition relationships"
    );

    // Verify transitions exist with correct source/target
    let transitions: Vec<_> = graph
        .relationships_by_kind(&RelationshipKind::Transition)
        .collect();
    assert_eq!(transitions.len(), 2);

    // Check that transitions connect the right steps
    let has_ignite_to_liftoff = transitions
        .iter()
        .any(|t| t.source == step1_id && t.target == step2_id);
    let has_liftoff_to_ascend = transitions
        .iter()
        .any(|t| t.source == step2_id && t.target == step3_id);

    assert!(has_ignite_to_liftoff, "Missing ignite->liftoff transition");
    assert!(has_liftoff_to_ascend, "Missing liftoff->ascend transition");

    // Verify idempotency
    let report2 = elaborate(&mut graph);
    assert_eq!(report2.successions_created, 0, "Second run should be no-op");
}

// ---------------------------------------------------------------------------
// C4: Flow connection pipeline
// ---------------------------------------------------------------------------

/// Build flow elements with endpoint children, elaborate, and verify properties.
#[test]
fn flow_elaborate_source_target_payload() {
    let mut graph = ModelGraph::new();

    // Flow usage
    let flow = Element::new_with_kind(ElementKind::FlowUsage).with_name("temperatureFlow");
    let flow_id = graph.add_element(flow);

    // Payload type (FeatureTyping child representing `of Temperature`)
    let typing = Element::new_with_kind(ElementKind::FeatureTyping)
        .with_owner(flow_id.clone())
        .with_prop("unresolved_type", "Temperature");
    graph.add_element(typing);

    // Source endpoint (sorted first by span)
    let src = Element::new_with_kind(ElementKind::Feature)
        .with_owner(flow_id.clone())
        .with_prop("isEnd", true)
        .with_name("sensor.reading")
        .with_span(Span::new("test", 0, 10));
    graph.add_element(src);

    // Target endpoint (sorted second by span)
    let tgt = Element::new_with_kind(ElementKind::Feature)
        .with_owner(flow_id.clone())
        .with_prop("isEnd", true)
        .with_name("controller.input")
        .with_span(Span::new("test", 10, 20));
    graph.add_element(tgt);

    // Elaborate
    let report = elaborate(&mut graph);

    assert_eq!(report.flows_derived, 1, "Should derive flow source/target");

    let elem = graph.get_element(&flow_id).unwrap();
    assert_eq!(
        elem.get_prop("source").and_then(|v| v.as_str()),
        Some("sensor.reading")
    );
    assert_eq!(
        elem.get_prop("target").and_then(|v| v.as_str()),
        Some("controller.input")
    );
    assert_eq!(
        elem.get_prop("payloadType").and_then(|v| v.as_str()),
        Some("Temperature")
    );
}

/// Flow with reference subsetting endpoints.
#[test]
fn flow_elaborate_reference_subsetting() {
    let mut graph = ModelGraph::new();

    let flow = Element::new_with_kind(ElementKind::FlowUsage).with_name("dataFlow");
    let flow_id = graph.add_element(flow);

    // Source endpoint with ReferenceSubsetting child
    let src_end = Element::new_with_kind(ElementKind::Feature)
        .with_owner(flow_id.clone())
        .with_prop("isEnd", true)
        .with_span(Span::new("test", 0, 10));
    let src_end_id = graph.add_element(src_end);

    let src_ref = Element::new_with_kind(ElementKind::ReferenceSubsetting)
        .with_owner(src_end_id)
        .with_prop("unresolved_subsettedFeature", "producer.output");
    graph.add_element(src_ref);

    // Target with direct name
    let tgt_end = Element::new_with_kind(ElementKind::Feature)
        .with_owner(flow_id.clone())
        .with_prop("isEnd", true)
        .with_name("consumer.input")
        .with_span(Span::new("test", 10, 20));
    graph.add_element(tgt_end);

    let report = elaborate(&mut graph);
    assert_eq!(report.flows_derived, 1);

    let elem = graph.get_element(&flow_id).unwrap();
    assert_eq!(
        elem.get_prop("source").and_then(|v| v.as_str()),
        Some("producer.output")
    );
    assert_eq!(
        elem.get_prop("target").and_then(|v| v.as_str()),
        Some("consumer.input")
    );
}

/// SuccessionFlowUsage — combined succession + flow.
#[test]
fn succession_flow_elaborate_pipeline() {
    let mut graph = ModelGraph::new();

    let flow = Element::new_with_kind(ElementKind::SuccessionFlowUsage).with_name("orderedFlow");
    let flow_id = graph.add_element(flow);

    let src = Element::new_with_kind(ElementKind::Feature)
        .with_owner(flow_id.clone())
        .with_prop("isEnd", true)
        .with_name("step1.out")
        .with_span(Span::new("test", 0, 10));
    graph.add_element(src);

    let tgt = Element::new_with_kind(ElementKind::Feature)
        .with_owner(flow_id.clone())
        .with_prop("isEnd", true)
        .with_name("step2.in")
        .with_span(Span::new("test", 10, 20));
    graph.add_element(tgt);

    let report = elaborate(&mut graph);
    assert_eq!(report.flows_derived, 1);

    let elem = graph.get_element(&flow_id).unwrap();
    assert_eq!(
        elem.get_prop("source").and_then(|v| v.as_str()),
        Some("step1.out")
    );
    assert_eq!(
        elem.get_prop("target").and_then(|v| v.as_str()),
        Some("step2.in")
    );
}
