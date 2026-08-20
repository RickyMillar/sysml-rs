// Build scripts panic on failure by convention
#![allow(clippy::expect_used)]
//! Build script for sysml-diagram: validates pipeline_coverage.toml at compile time.
//!
//! Checks that every VisualKind and CompartmentKind variant has an entry
//! in the TOML tracking file, and prints coverage statistics.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

fn main() {
    let toml_path = Path::new("src/pipeline_coverage.toml");
    println!("cargo:rerun-if-changed=src/pipeline_coverage.toml");
    println!("cargo:rerun-if-changed=src/visual_kind.rs");

    if !toml_path.exists() {
        println!("cargo:warning=pipeline_coverage.toml not found — skipping coverage check");
        return;
    }

    let toml_content =
        fs::read_to_string(toml_path).expect("Failed to read pipeline_coverage.toml");
    let toml: toml::Value = toml_content
        .parse()
        .expect("Failed to parse pipeline_coverage.toml");

    // --- Node coverage ---
    let nodes = toml
        .get("node")
        .and_then(|v| v.as_array())
        .expect("pipeline_coverage.toml missing [[node]] array");

    let node_names: HashSet<String> = nodes
        .iter()
        .filter_map(|n| n.get("name").and_then(|v| v.as_str()).map(String::from))
        .collect();

    // Expected VisualKind variants (must match visual_kind.rs enum)
    let expected_nodes: Vec<&str> = vec![
        "Package",
        "Part",
        "Item",
        "Connection",
        "Action",
        "State",
        "Constraint",
        "Calculation",
        "Requirement",
        "Concern",
        "VerificationCase",
        "UseCase",
        "AnalysisCase",
        "Interface",
        "Attribute",
        "Enumeration",
        "Allocation",
        "Occurrence",
        "Flow",
        "View",
        "Viewpoint",
        "Port",
        "Rendering",
        "Comment",
        "Metadata",
        "Actor",
        "InitialNode",
        "FinalNode",
        "DecisionNode",
        "MergeNode",
        "ForkNode",
        "JoinNode",
        "TerminateNode",
        "SendAction",
        "AcceptAction",
        "Lifeline",
        "SqProxy",
        "NaryDot",
        "Generic",
    ];

    let mut missing_nodes = Vec::new();
    for name in &expected_nodes {
        if !node_names.contains(*name) {
            missing_nodes.push(*name);
        }
    }
    if !missing_nodes.is_empty() {
        println!(
            "cargo:warning=pipeline_coverage.toml missing node entries: {}",
            missing_nodes.join(", ")
        );
    }

    // --- Edge coverage ---
    let edges = toml
        .get("edge")
        .and_then(|v| v.as_array())
        .expect("pipeline_coverage.toml missing [[edge]] array");

    let edge_names: HashSet<String> = edges
        .iter()
        .filter_map(|e| e.get("name").and_then(|v| v.as_str()).map(String::from))
        .collect();

    let expected_edges: Vec<&str> = vec![
        "Owning",
        "TypeOf",
        "Satisfy",
        "Verify",
        "Derive",
        "Trace",
        "Reference",
        "Specialize",
        "Redefine",
        "Subsetting",
        "Flow",
        "Transition",
        "Dependency",
        "Import",
        "Allocate",
        "Binding",
        "Connection",
        "Perform",
        "Exhibit",
        "Include",
        "Succession",
        "Composition",
        "Annotation",
        "SuccessionFlow",
        "Message",
        "FeatureMembership",
        "Membership",
        "FlowOnConnection",
        "InterfaceConnection",
        "Portion",
        "Expose",
        "Frame",
        "Assert",
        "Assume",
        "Require",
        "ParameterLink",
        "EventOccurrence",
    ];

    let mut missing_edges = Vec::new();
    for name in &expected_edges {
        if !edge_names.contains(*name) {
            missing_edges.push(*name);
        }
    }
    if !missing_edges.is_empty() {
        println!(
            "cargo:warning=pipeline_coverage.toml missing edge entries: {}",
            missing_edges.join(", ")
        );
    }

    // --- Compartment coverage ---
    let compartments = toml
        .get("compartment")
        .and_then(|v| v.as_array())
        .expect("pipeline_coverage.toml missing [[compartment]] array");

    // --- Coverage statistics ---
    let count_status = |items: &[toml::Value], field: &str| -> (usize, usize, usize) {
        let mut done = 0;
        let mut partial = 0;
        let total = items.len();
        for item in items {
            if let Some(s) = item.get(field).and_then(|v| v.as_str()) {
                match s {
                    "done" => done += 1,
                    "partial" => partial += 1,
                    _ => {}
                }
            }
        }
        (done, partial, total)
    };

    let (ts_done, ts_partial, ts_total) = count_status(nodes, "ts_reg");
    let (edge_ts_done, edge_ts_partial, edge_ts_total) = count_status(edges, "ts_reg");

    // Count generator coverage across all views
    let view_names = [
        "General",
        "Interconnection",
        "StateTransition",
        "ActionFlow",
        "Browser",
        "Sequence",
        "Grid",
        "Geometry",
    ];
    let mut gen_done = 0usize;
    let mut gen_total = 0usize;
    for node in nodes {
        if let Some(gens) = node.get("generators").and_then(|v| v.as_table()) {
            for view in &view_names {
                if let Some(status) = gens.get(*view).and_then(|v| v.as_str()) {
                    if status != "n/a" {
                        gen_total += 1;
                        if status == "done" {
                            gen_done += 1;
                        }
                    }
                }
            }
        }
    }

    println!("cargo:warning=Pipeline coverage: {}/{} nodes TS-registered, {}/{} edges TS-registered, {} compartments tracked",
        ts_done + ts_partial, ts_total,
        edge_ts_done + edge_ts_partial, edge_ts_total,
        compartments.len());
    println!(
        "cargo:warning=Generator coverage: {}/{} (node × view) pairs done (excluding n/a)",
        gen_done, gen_total
    );

    // Count crash risks
    let crash_nodes: Vec<&str> = nodes
        .iter()
        .filter(|n| n.get("ts_reg").and_then(|v| v.as_str()) == Some("missing"))
        .filter_map(|n| n.get("name").and_then(|v| v.as_str()))
        .collect();
    let crash_edges: Vec<&str> = edges
        .iter()
        .filter(|e| e.get("ts_reg").and_then(|v| v.as_str()) == Some("missing"))
        .filter_map(|e| e.get("name").and_then(|v| v.as_str()))
        .collect();

    if !crash_nodes.is_empty() || !crash_edges.is_empty() {
        println!(
            "cargo:warning=CRASH RISK: {} node types + {} edge types missing TS registration",
            crash_nodes.len(),
            crash_edges.len()
        );
    }
}
