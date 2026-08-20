//! Contract: `sysml.diagram.viewmodel` exposes the renderer-agnostic ViewModel
//! (Bucket 1.7, re-keyed in Bucket 2 F2) — scene + design tokens + text-map +
//! interaction descriptors + frame — as JSON across the service layer.
//!
//! Post-F2 the command renders a **declared view** (ViewUsage / ViewDefinition)
//! scoped by its Expose / filter memberships, mirroring `sysml.views.render`.
//! There is no ad-hoc `view_type`-only projection (that silently dumped the whole
//! elaborated graph incl. the standard library — steward-ruled removed).

use std::collections::HashSet;
use std::path::PathBuf;

use sysml_core::ElementId;
use sysml_project::discovery::OpenTarget;
use sysml_service::SysmlService;

fn coffee_machine_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/book-examples/coffee-machine")
        .canonicalize()
        .expect("coffee-machine fixture must exist")
}

/// Pick a declared view that actually exposes elements (so the scene is scoped
/// and a frame is produced). Falls back to any view if none expose.
fn exposing_view_id(svc: &SysmlService) -> ElementId {
    let summaries = svc.views_list("__workspace__").expect("views_list");
    summaries
        .iter()
        .find(|s| !s.exposed.is_empty())
        .or_else(|| summaries.first())
        .map(|s| s.id.clone())
        .expect("coffee-machine fixture must declare at least one view")
}

#[test]
fn viewmodel_command_returns_scene_tokens_interactions_and_frame() {
    let svc = SysmlService::empty();
    svc.open_context(OpenTarget::Folder(coffee_machine_root()))
        .expect("coffee-machine open_context should succeed");

    let view_id = exposing_view_id(&svc);
    let expanded: HashSet<String> = HashSet::new();
    let value = svc
        .diagram_view_model("__workspace__", &view_id, &expanded)
        .expect("viewmodel command succeeds for a declared view");

    // The promoted DiagramIR scene is present and populated.
    let scene = value.get("scene").expect("ViewModel carries a scene");
    let nodes = scene
        .get("nodes")
        .and_then(|n| n.as_array())
        .expect("scene has a nodes array");
    assert!(!nodes.is_empty(), "scene should contain the view's nodes");

    // F2 — the scene is SCOPED by the view's exposes, not the whole elaborated
    // graph. The unscoped ad-hoc projection dumped the entire standard library
    // (95+ top-level nodes incl. IntegerFunctions / ScalarValues / …). A scoped
    // declared view must be small and must not surface std-library packages.
    assert!(
        nodes.len() < 50,
        "scoped view should have few top-level nodes, got {}",
        nodes.len()
    );
    let names: Vec<&str> = nodes.iter().filter_map(|n| n.get("name").and_then(|x| x.as_str())).collect();
    for lib in ["IntegerFunctions", "ScalarValues", "ISQ", "KerML"] {
        assert!(!names.contains(&lib), "scoped scene must not include std-library package {lib}");
    }

    // Design tokens (1.3) — the color palette — are attached.
    let palette = value
        .get("tokens")
        .and_then(|t| t.get("palette"))
        .expect("ViewModel carries design tokens with a palette");
    assert!(
        palette.get("block").is_some(),
        "palette should expose node-category colors"
    );

    // Interaction descriptors (1.5) + text-map (1.6) are attached.
    assert!(value.get("interactions").is_some(), "ViewModel carries interaction descriptors");
    assert!(value.get("text_map").is_some(), "ViewModel carries the ElementId<->Span text-map");

    // §F-10 frame: a DECLARED view is a framed view, so `frame` is populated
    // (not null) — the key F2 signal that scoping worked.
    let frame = value.get("frame").expect("ViewModel has a frame field");
    assert!(!frame.is_null(), "a declared view must carry a framed-view (frame != null)");
}

#[test]
fn viewmodel_command_errors_on_unknown_view() {
    let svc = SysmlService::empty();
    svc.open_context(OpenTarget::Folder(coffee_machine_root()))
        .expect("coffee-machine open_context should succeed");
    let bogus = ElementId::from_string("synthetic-nonexistent-view-id");
    let expanded: HashSet<String> = HashSet::new();
    let err = svc.diagram_view_model("__workspace__", &bogus, &expanded);
    assert!(err.is_err(), "unknown view id should fail hard, not return empty");
}

#[test]
fn viewmodel_command_fails_hard_on_empty_service() {
    let svc = SysmlService::empty();
    let view_id = ElementId::from_string("any");
    let expanded: HashSet<String> = HashSet::new();
    let err = svc.diagram_view_model("__workspace__", &view_id, &expanded);
    assert!(err.is_err(), "no workspace loaded must fail hard");
}
