//! Constraint-block (parametric) notation tests for the Interconnection
//! generator — findings register R2-7.
//!
//! Notation contract (sysml-graphical-notation-contract.md, constraint /
//! parametric section; see the crate README on constraint notation): a constraint
//! block shows its `{expression}` in a compartment and its `in` parameters as
//! small square parametric ports (`PortTag::Parametric`). Both corpus
//! authoring shapes must produce content:
//!   • constraint-with-parameters style — named params (`in x : Real;`) + a
//!     trailing expression. Params parse as `ReferenceUsage` with a `direction` prop,
//!     which the old port gate (AttributeUsage-only) missed → no ports.
//!   • view-showcase style — no expression at all, only a `doc /* ... */`
//!     body. Nothing rendered → name-only block. The doc body now renders as
//!     compartment text (NOT brace-wrapped: a doc is an annotation, not an
//!     expression).

mod smodel_common;
use smodel_common::parse_sysml;

use sysml_diagram::ir::types::{DiagramChild, DiagramIR, DiagramNode, PortSide, PortTag};
use sysml_diagram::smodel::ViewType;
use sysml_diagram::ViewRequest;

/// A FaradayLaw constraint authoring shape — named params plus a trailing
/// expression — verbatim minus the ISQ/SI imports it doesn't need.
const PARAMETRIC_SHAPE: &str = "
    package Physics {
        constraint def FaradayLaw {
            doc /* V_net = N * Ae * dB/dt */
            in V_net : Real;
            in N : Real;
            in Ae : Real;
            in dBdt : Real;
            dBdt == V_net / (N * Ae)
        }
    }
";

/// view-showcase/Model.sysml authoring shape (MaxPowerConstraint, verbatim):
/// no params, no expression — the "equation" lives only in the doc body.
const SHOWCASE_SHAPE: &str = "
    package Showcase {
        constraint def MaxPowerConstraint {
            doc /* engine.power <= 200 */
        }
    }
";

/// Render an Interconnection view exposing the named constraint def and
/// return the scene (parametric diagram = Interconnection + constraint
/// exposes, per requirements-parametric-retirement.md §1.4).
fn render_exposed_constraint(source: &str, name: &str) -> DiagramIR {
    let mut graph = parse_sysml(source);
    sysml_core::elaborate::elaborate(&mut graph);
    let id = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("{name} present in graph"))
        .id
        .clone();
    let request = ViewRequest::new(ViewType::Interconnection).with_exposes(vec![id]);
    let vm = sysml_diagram::to_view_model(&graph, &request);
    (*vm.scene).clone()
}

fn find_node<'a>(scene: &'a DiagramIR, name: &str) -> &'a DiagramNode {
    scene
        .nodes
        .iter()
        .find(|n| n.name.contains(name))
        .unwrap_or_else(|| {
            let names: Vec<_> = scene.nodes.iter().map(|n| n.name.clone()).collect();
            panic!("constraint block {name} must render; got {names:?}")
        })
}

fn text_children(node: &DiagramNode) -> Vec<String> {
    node.children
        .iter()
        .filter_map(|c| match c {
            DiagramChild::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

// ── (1) view-showcase shape: doc-only constraint def renders its text ────

#[test]
fn doc_only_constraint_def_renders_doc_text() {
    let scene = render_exposed_constraint(SHOWCASE_SHAPE, "MaxPowerConstraint");
    let node = find_node(&scene, "MaxPowerConstraint");
    let texts = text_children(node);
    assert!(
        texts.iter().any(|t| t.contains("engine.power <= 200")),
        "doc-only constraint block must not be name-only — the doc body \
         (its only content) must render as compartment text; got {texts:?}"
    );
    // A doc body is NOT an expression — it must not be brace-wrapped.
    assert!(
        !texts.iter().any(|t| t.contains("{engine.power <= 200}")),
        "doc text must not masquerade as a {{expression}}; got {texts:?}"
    );
}

// ── (2) parametric shape: one parametric port per `in` parameter ─────────

#[test]
fn in_parameters_emit_one_parametric_port_each() {
    let scene = render_exposed_constraint(PARAMETRIC_SHAPE, "FaradayLaw");
    let node = find_node(&scene, "FaradayLaw");

    let mut port_names: Vec<&str> = node.ports.iter().map(|p| p.name.as_str()).collect();
    port_names.sort_unstable();
    assert_eq!(
        port_names,
        vec!["Ae", "N", "V_net", "dBdt"],
        "each `in` parameter emits exactly one port"
    );
    for port in &node.ports {
        assert!(
            port.tags.contains(&PortTag::Parametric),
            "parameter port {} must carry PortTag::Parametric; tags {:?}",
            port.name,
            port.tags
        );
        assert_eq!(
            port.size,
            Some((8.0, 8.0)),
            "parameter port {} must be a small 8x8 square",
            port.name
        );
        assert_eq!(
            port.side,
            Some(PortSide::West),
            "`in` parameter port {} sits on the input (west) side",
            port.name
        );
    }
}

// ── (3) parametric shape: equation compartment unchanged (byte-accurate) ─

#[test]
fn parametric_equation_text_still_byte_accurate() {
    let scene = render_exposed_constraint(PARAMETRIC_SHAPE, "FaradayLaw");
    let node = find_node(&scene, "FaradayLaw");
    let texts = text_children(node);
    assert!(
        texts.iter().any(|t| t == "{dBdt == V_net / (N * Ae)}"),
        "equation must still render byte-accurate in braces; got {texts:?}"
    );
    // The doc body must NOT displace the expression: when a constraint has a
    // real expression, only the expression renders (physicsEquations parity).
    assert!(
        !texts.iter().any(|t| t.contains("V_net = N * Ae * dB/dt")),
        "doc fallback must not fire when the constraint has an expression; got {texts:?}"
    );
}

// ── End-to-end on the actual view-showcase fixture ───────────────────────

#[test]
fn view_showcase_fixture_constraint_view_has_content() {
    // ConstraintView in examples/view-showcase/Views.sysml exposes
    // MaxPowerConstraint + TorqueBalance; both blocks must carry content
    // (this was the name-only S1 in R2-7).
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../examples/view-showcase/Model.sysml"
    ))
    .expect("view-showcase Model.sysml readable");
    let mut graph = parse_sysml(&source);
    sysml_core::elaborate::elaborate(&mut graph);

    let find = |name: &str| {
        graph
            .elements
            .values()
            .find(|e| e.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("{name} present"))
            .id
            .clone()
    };
    let request = ViewRequest::new(ViewType::Interconnection)
        .with_exposes(vec![find("MaxPowerConstraint"), find("TorqueBalance")]);
    let vm = sysml_diagram::to_view_model(&graph, &request);

    for (name, body) in [
        ("MaxPowerConstraint", "engine.power <= 200"),
        ("TorqueBalance", "engine.torqueOut == gearbox.torqueIn"),
    ] {
        let node = find_node(&vm.scene, name);
        let texts = text_children(node);
        assert!(
            texts.iter().any(|t| t.contains(body)),
            "{name} must render its doc body {body:?}; got {texts:?}"
        );
    }
}
