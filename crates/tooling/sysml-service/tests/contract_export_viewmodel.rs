//! Contract: `SysmlService::export_view_model` — the export path behind the
//! CLI `sysml export viewmodel` (fixture baking for diagram embeds).
//!
//! Same request composition as `sysml.diagram.viewmodel`, plus sidecar pruning:
//! the whole-graph `text_map` / `interactions` maps are scoped to the ids the
//! scene / non-graph payload references (`ViewModel::pruned_to_referenced`).
//! Serialized unpruned they carry a span for every workspace element — the
//! embed spike measured megabytes for a view whose scene held a handful of
//! nodes.

use std::collections::HashSet;
use std::path::PathBuf;

use sysml_core::ElementId;
use sysml_project::discovery::OpenTarget;
use sysml_service::SysmlService;

fn view_showcase_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../examples/view-showcase")
        .canonicalize()
        .expect("view-showcase example must exist")
}

fn view_id_by_name(svc: &SysmlService, name: &str) -> ElementId {
    svc.views_list("__workspace__")
        .expect("views_list")
        .iter()
        .find(|s| s.name.as_deref() == Some(name))
        .map(|s| s.id.clone())
        .unwrap_or_else(|| panic!("view-showcase must declare {name}"))
}

/// The keys a pruned text-map retains, as owned strings.
fn text_map_keys(value: &serde_json::Value) -> Vec<String> {
    value
        .get("text_map")
        .and_then(|tm| tm.get("spans"))
        .and_then(|s| s.as_object())
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

/// One workspace load covers both view families (the load dominates runtime).
#[test]
fn export_viewmodel_prunes_sidecars_for_graph_and_browser_views() {
    let svc = SysmlService::empty();
    svc.open_context(OpenTarget::Folder(view_showcase_root()))
        .expect("open view-showcase example");
    let expanded: HashSet<String> = HashSet::new();

    // ── Graph view (OverviewView: General, expose Vehicle) ──────────────
    let overview = view_id_by_name(&svc, "OverviewView");
    let raw = svc
        .diagram_view_model("__workspace__", &overview, &expanded)
        .expect("unpruned viewmodel renders");
    let pruned = svc
        .export_view_model(&overview, &expanded, false)
        .expect("pruned export renders");

    // Stable schema: same top-level keys as the live wire artifact.
    let mut keys: Vec<&str> = pruned
        .as_object()
        .expect("ViewModel serializes as an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["frame", "interactions", "non_graph", "scene", "text_map", "tokens"],
        "export must not change the ViewModel schema"
    );

    // Scene is present and populated.
    let nodes = pruned["scene"]["nodes"]
        .as_array()
        .expect("scene has a nodes array");
    assert!(!nodes.is_empty(), "graph view scene must be non-empty");

    // Pruned text_map ⊆ scene ids: every retained span key is referenced by
    // the serialized scene (ids are UUID strings, so containment is exact).
    let scene_json = serde_json::to_string(&pruned["scene"]).expect("scene serializes");
    let pruned_keys = text_map_keys(&pruned);
    assert!(!pruned_keys.is_empty(), "scene-backed spans survive pruning");
    for key in &pruned_keys {
        assert!(
            scene_json.contains(key),
            "pruned text_map key {key} is not referenced by the scene"
        );
    }

    // Pruning actually bites: the raw map spans the whole workspace
    // (user model + standard library), the pruned map only this view.
    let raw_keys = text_map_keys(&raw);
    assert!(
        pruned_keys.len() * 10 < raw_keys.len(),
        "pruned map ({}) should be a small fraction of the whole-graph map ({})",
        pruned_keys.len(),
        raw_keys.len()
    );
    let raw_bytes = serde_json::to_string(&raw).expect("raw serializes").len();
    let pruned_bytes = serde_json::to_string(&pruned).expect("pruned serializes").len();
    eprintln!("OverviewView export: raw {raw_bytes} bytes, pruned {pruned_bytes} bytes");
    assert!(
        pruned_bytes < raw_bytes,
        "pruned export must be smaller than the raw wire artifact"
    );

    // ── Non-graph view (CatalogView: BrowserView, expose Showcase::*) ────
    let catalog = view_id_by_name(&svc, "CatalogView");
    let pruned = svc
        .export_view_model(&catalog, &expanded, false)
        .expect("browser export renders");
    let non_graph = &pruned["non_graph"];
    assert_eq!(
        non_graph["kind"].as_str(),
        Some("tree"),
        "BrowserView exports the tree non-graph payload"
    );
    assert!(
        !non_graph["data"]["roots"].as_array().expect("tree has roots").is_empty(),
        "browser tree must be non-empty"
    );

    // For a non-graph view the sidecars join the payload, not the (empty)
    // scene: every retained span key is referenced by the tree.
    let tree_json = serde_json::to_string(non_graph).expect("tree serializes");
    let keys = text_map_keys(&pruned);
    assert!(!keys.is_empty(), "tree-backed spans survive pruning");
    for key in &keys {
        assert!(
            tree_json.contains(key),
            "pruned text_map key {key} is not referenced by the tree payload"
        );
    }
}
