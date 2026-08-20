//! SequenceView integration tests for the SModel visualization pipeline.

mod smodel_common;
use smodel_common::*;

use sysml_diagram::smodel::{SGraph, SModelElement, ViewType};

#[test]
fn sequence_empty_no_crash() {
    let sg = generate("package P { part def A; }", ViewType::Sequence, false);
    assert_eq!(sg.type_, "graph");
}

#[test]
fn sequence_with_actions() {
    let sg = generate(
        "package P { action def Client; action def Server; action c : Client; action s : Server; }",
        ViewType::Sequence,
        false,
    );
    assert_eq!(sg.type_, "graph");
}

#[test]
fn sequence_serializes() {
    let sg = generate("package P { action def A; }", ViewType::Sequence, false);
    let json = serde_json::to_string(&sg);
    assert!(json.is_ok());
}

// ── Corpus-driven: view-showcase CommsView (expose Vehicle) ──────────────
//
// R2-9 regression gates: the showcase's `CommsView :> Sequence { expose
// Vehicle; }` must render a REAL sequence — typed lifeline heads, a message
// arrow for the corpus `flow torqueFlow of Torque` exchange, and no
// occurrence dot left without an incident message edge.

/// Parse the view-showcase corpus and render the Sequence view scoped to
/// `Vehicle` — exactly what the declared `CommsView` renders.
fn generate_comms_view() -> SGraph {
    use sysml_parser_incremental::TreeSitterParser;
    use sysml_parser_trait::{Parser, SysmlFile};

    let root = env!("CARGO_MANIFEST_DIR");
    let model =
        std::fs::read_to_string(format!("{root}/../../../examples/view-showcase/Model.sysml"))
            .expect("read Model.sysml");
    let views =
        std::fs::read_to_string(format!("{root}/../../../examples/view-showcase/Views.sysml"))
            .expect("read Views.sysml");

    let parser = TreeSitterParser::new();
    let files = vec![
        SysmlFile::new("Model.sysml", &model),
        SysmlFile::new("Views.sysml", &views),
    ];
    let result = parser.parse(&files);
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.severity == sysml_span::Severity::Error)
        .collect();
    assert!(errors.is_empty(), "corpus parse errors: {errors:?}");
    let mut graph = result.graph;
    sysml_core::elaborate::elaborate(&mut graph);

    let vehicle_id = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("Vehicle"))
        .map(|e| e.id.clone())
        .expect("Vehicle element in showcase corpus");

    let request = sysml_diagram::ViewRequest::new(ViewType::Sequence).with_expose(vehicle_id);
    sysml_diagram::smodel::to_smodel_with(&graph, &request)
}

/// Recursively collect `(node_type, node_id, name_label_text)` for all nodes.
fn collect_nodes(children: &[SModelElement], out: &mut Vec<(String, String, Option<String>)>) {
    for child in children {
        if let SModelElement::Node(n) = child {
            let name = n.children.iter().find_map(|c| match c {
                SModelElement::Label(l) if l.type_ == "label:name" => Some(l.text.clone()),
                _ => None,
            });
            out.push((n.type_.clone(), n.id.clone(), name));
            collect_nodes(&n.children, out);
        }
    }
}

/// Collect `(edge_id, label_text)` for all top-level message-family edges.
fn collect_message_edges(sg: &SGraph) -> Vec<(String, Option<String>)> {
    sg.children
        .iter()
        .filter_map(|c| match c {
            SModelElement::Edge(e)
                if e.type_.starts_with("edge:message")
                    || e.type_.starts_with("edge:flow")
                    || e.type_.starts_with("edge:succession") =>
            {
                let label = e.children.iter().find_map(|ec| match ec {
                    SModelElement::Label(l) if l.type_ == "label:edge" => Some(l.text.clone()),
                    _ => None,
                });
                Some((e.id.clone(), label))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn comms_view_lifeline_heads_carry_keyword_and_type() {
    let sg = generate_comms_view();
    let mut nodes = Vec::new();
    collect_nodes(&sg.children, &mut nodes);

    let head = |suffix: &str| {
        nodes
            .iter()
            .find(|(t, id, _)| t == "node:lifeline" && id.ends_with(suffix))
            .unwrap_or_else(|| panic!("missing lifeline {suffix}"))
            .2
            .clone()
            .expect("lifeline head has a name label")
    };

    // Contract sequence section: head reads `«part» name : Type`.
    assert_eq!(head("lifeline:engine"), "\u{00ab}part\u{00bb} engine : Engine");
    assert_eq!(
        head("lifeline:gearbox"),
        "\u{00ab}part\u{00bb} gearbox : Gearbox"
    );
}

#[test]
fn comms_view_renders_torque_flow_message_edge() {
    let sg = generate_comms_view();
    let message_edges = collect_message_edges(&sg);

    assert!(
        !message_edges.is_empty(),
        "CommsView must render at least one message edge (R2-9: none rendered)"
    );

    // The corpus `flow torqueFlow of Torque from engine.torqueOut to
    // gearbox.torqueIn` must surface as a message arrow whose ON-ARROW label
    // is the flow's declared name (contract §B end-label: UsageDeclaration +
    // optional `of <payload>`). NOTE: the payload suffix (`of Torque`) is
    // currently absent because the tree-sitter lowering drops the flow's
    // `of <type_ref>` clause (no FeatureTyping child is minted, so elaborate
    // never sets `payloadType`) — a parser gap, not a generator choice. When
    // that gap closes this label becomes `torqueFlow of Torque` and this
    // assertion (contains) keeps passing.
    assert!(
        message_edges
            .iter()
            .any(|(_, l)| l.as_deref().is_some_and(|t| t.contains("torqueFlow"))),
        "expected a message edge labeled with the torqueFlow exchange, got: {message_edges:?}"
    );

    // The declared connector surfaces too (RSC-3.5c.2b), under its own name —
    // the two exchanges must be distinguishable on the wire labels.
    assert!(
        message_edges
            .iter()
            .any(|(_, l)| l.as_deref().is_some_and(|t| t.contains("engineToGearbox"))),
        "expected the declared connector edge to surface, got: {message_edges:?}"
    );
}

#[test]
fn comms_view_has_no_orphan_occurrence_dots() {
    let sg = generate_comms_view();
    let mut nodes = Vec::new();
    collect_nodes(&sg.children, &mut nodes);

    let proxy_ids: Vec<&String> = nodes
        .iter()
        .filter(|(t, _, _)| t == "node:sqProxy")
        .map(|(_, id, _)| id)
        .collect();
    assert!(
        !proxy_ids.is_empty(),
        "expected occurrence proxies in the scene"
    );

    let mut endpoint_ids = std::collections::HashSet::new();
    for c in &sg.children {
        if let SModelElement::Edge(e) = c {
            endpoint_ids.insert(e.source_id.clone());
            endpoint_ids.insert(e.target_id.clone());
        }
    }

    // R2-9: no dot may render without an incident message arrow.
    for id in proxy_ids {
        assert!(
            endpoint_ids.contains(id),
            "occurrence dot {id} has no incident message edge (orphan dot)"
        );
    }
}

#[test]
fn comms_view_message_route_has_on_arrow_label_anchor() {
    let sg = generate_comms_view();
    // Non-self message routes carry an explicit midpoint so fixed-layout
    // renderers place the label ON the arrow (D-N5), not at the sender.
    let mut saw_route = false;
    for c in &sg.children {
        if let SModelElement::Edge(e) = c {
            if let Some(route) = &e.precomputed_route {
                saw_route = true;
                assert!(
                    route.len() >= 3,
                    "message route must include a midpoint label anchor, got {route:?}"
                );
            }
        }
    }
    assert!(saw_route, "expected at least one precomputed message route");
}
