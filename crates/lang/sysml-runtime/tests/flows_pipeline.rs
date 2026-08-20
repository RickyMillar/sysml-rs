//! End-to-end pipeline tests: build ModelGraph -> elaborate -> compile -> route.
//!
//! These tests verify the full flow routing pipeline from programmatic
//! graph construction through elaboration, compilation to IR, and routing.

use sysml_core::elaborate::elaborate;
use sysml_core::{Element, ElementKind, ModelGraph, Value};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::exchange::ExchangePlane;
use sysml_runtime::flows::compile_ports;
use sysml_runtime::links::{classify_links, LinkIR, LinkSourceKind};

/// RSC-3.5e.5 W4: `compile_flows` is gone — the producer is folded into
/// `classify_links`. The FlowUsage subset of the classified `LinkGraph` is the
/// former flow list (interning order preserved); `LinkEndpoint.owner`/`port`
/// mirror the old `FlowEndpoint.participant`/`port`.
fn flow_links(graph: &ModelGraph) -> Vec<LinkIR> {
    let reg = compile_ports(graph);
    let (lg, _diags) = classify_links(graph, &reg);
    lg.iter()
        .filter(|l| l.kind == LinkSourceKind::FlowUsage)
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// Flow pipeline tests
// ---------------------------------------------------------------------------

/// Build a flow graph with source/target properties, elaborate, compile, and route.
///
/// Graph: FlowUsage "dataFlow" with source=sensor.output, target=controller.input
#[test]
fn flow_elaborate_compile_route() {
    let mut graph = ModelGraph::new();

    // Create a flow usage with source and target
    let flow = Element::new_with_kind(ElementKind::FlowUsage)
        .with_name("dataFlow")
        .with_prop("source", "sensor.output")
        .with_prop("target", "controller.input");
    graph.add_element(flow);

    // Elaborate (may derive additional properties)
    let report = elaborate(&mut graph);
    eprintln!("Flow elaboration: {}", report);

    // Compile flows from graph (via the classified link graph)
    let flows = flow_links(&graph);
    assert_eq!(flows.len(), 1, "Should compile 1 flow");
    assert_eq!(flows[0].source.owner, "sensor");
    assert_eq!(flows[0].source.port, "output");
    assert_eq!(flows[0].target.owner, "controller");
    assert_eq!(flows[0].target.port, "input");

    // Set up router and route a message
    let mut router = ExchangePlane::new();
    for link in flows {
        let id = link.display_label(&graph);
        router.add_flow(link, id);
    }

    router.send("sensor.output", Value::Float(25.5));
    let delivered = router.route_pending();

    assert_eq!(delivered.len(), 1, "Should deliver 1 message");
    assert_eq!(delivered[0].target, "controller.input");
    assert_eq!(delivered[0].payload, Value::Float(25.5));

    // Receive the message
    let msg = router.receive("controller.input");
    assert!(msg.is_some(), "Should have message in delivery queue");
}

/// Multiple flows with different endpoints.
#[test]
fn multiple_flows_route_independently() {
    let mut graph = ModelGraph::new();

    let flow1 = Element::new_with_kind(ElementKind::FlowUsage)
        .with_name("tempFlow")
        .with_prop("source", "sensor.tempOut")
        .with_prop("target", "monitor.tempIn");
    graph.add_element(flow1);

    let flow2 = Element::new_with_kind(ElementKind::FlowUsage)
        .with_name("pressureFlow")
        .with_prop("source", "sensor.pressOut")
        .with_prop("target", "monitor.pressIn");
    graph.add_element(flow2);

    elaborate(&mut graph);

    let flows = flow_links(&graph);
    assert_eq!(flows.len(), 2, "Should compile 2 flows");

    let mut router = ExchangePlane::new();
    for link in flows {
        let id = link.display_label(&graph);
        router.add_flow(link, id);
    }

    // Send on both flows
    router.send("sensor.tempOut", Value::Float(100.0));
    router.send("sensor.pressOut", Value::Float(1.5));

    let delivered = router.route_pending();
    assert_eq!(delivered.len(), 2, "Should deliver 2 messages");

    // Each target should receive its own message
    let temp_msg = router.receive("monitor.tempIn");
    assert!(temp_msg.is_some());
    assert_eq!(temp_msg.unwrap().payload, Value::Float(100.0));

    let press_msg = router.receive("monitor.pressIn");
    assert!(press_msg.is_some());
    assert_eq!(press_msg.unwrap().payload, Value::Float(1.5));
}

/// Succession flow blocks until source is marked complete.
#[test]
fn succession_flow_blocks_until_complete() {
    let mut graph = ModelGraph::new();

    let flow = Element::new_with_kind(ElementKind::SuccessionFlowUsage)
        .with_name("orderedFlow")
        .with_prop("source", "step1.out")
        .with_prop("target", "step2.in");
    graph.add_element(flow);

    elaborate(&mut graph);

    let flows = flow_links(&graph);
    assert_eq!(flows.len(), 1);
    assert!(flows[0].is_succession, "Should be a succession flow");

    let mut router = ExchangePlane::new();
    for link in flows {
        let id = link.display_label(&graph);
        router.add_flow(link, id);
    }

    // Send before completion — should be deferred
    router.send("step1.out", Value::String("result".into()));
    let delivered = router.route_pending();
    assert_eq!(
        delivered.len(),
        0,
        "Succession flow should block before source completes"
    );

    // Mark source as complete and re-route
    router.mark_completed("step1");
    let delivered = router.route_pending();
    assert_eq!(
        delivered.len(),
        1,
        "Succession flow should deliver after source completes"
    );
}

// ---------------------------------------------------------------------------
// Parsed .sysml pipeline test
// ---------------------------------------------------------------------------

/// Parse a .sysml string containing flow connections, elaborate, and compile.
#[test]
fn parsed_sysml_flow_pipeline() {
    let source = r#"
        package FlowExample {
            part def Sensor;
            part def Controller;

            flow sensorData from sensor.output to controller.input;
        }
    "#;

    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("test.sysml", source)]);

    // Note: flow syntax may produce parse diagnostics depending on parser coverage
    eprintln!(
        "Parsed flow: {} elements, {} diagnostics",
        result.graph.element_count(),
        result.diagnostics.len()
    );

    // Elaborate
    let report = elaborate(&mut result.graph);
    eprintln!("Flow elaboration: {}", report);

    // Check for flow-like elements
    let flow_usages: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::FlowUsage)
        .collect();

    eprintln!("FlowUsage elements found: {}", flow_usages.len());

    // If flows were parsed, classify them into the link graph.
    if !flow_usages.is_empty() {
        let flows = flow_links(&result.graph);
        eprintln!("Compiled {} flows", flows.len());
        for f in &flows {
            eprintln!(
                "  {} -> {} (succession={})",
                f.source.key(),
                f.target.key(),
                f.is_succession
            );
        }
    }
}
