//! Contract: `sysml.diagram.diagnostic_overlay` (F16-2) returns a well-formed,
//! ElementId-keyed diagnostics overlay for a declared view's scene, and does not
//! error on a workspace open (regression gate for the `__workspace__` diagnostics
//! bug — the sentinel is a graph-accessor key, not a diagnostics file id, so the
//! command must aggregate over the loaded user files instead).
//!
//! Scope note (findings from the live populate-check, 2026-07-14): the
//! span→element→scene join is mechanically sound and identity-consistent — a
//! diagnostic's span resolves via the ide-db position map to a real element that
//! lives in the SAME workspace-graph id-space as the scene's node ids (so
//! ElementId propagation through the diagram is intact for this path). Whether a
//! badge actually lands additionally requires the declared view to *render* the
//! diagnosed element; a supertype-less `view def` renders a fallback scene that
//! need not include the diagnosed element, in which case the overlay is honestly
//! empty (sparse, scene-scoped — never a fabricated placement). Asserting a
//! populated badge end-to-end therefore depends on view-generation behaviour
//! outside this command's contract, so this gate asserts the command's own
//! guarantees: it succeeds and returns the sparse ElementId-keyed shape.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use sysml_core::ElementId;
use sysml_project::discovery::OpenTarget;
use sysml_service::SysmlService;

/// A declared view plus a part with a duplicate name — a structural
/// (always-published) diagnostic, so the workspace has real diagnostics to feed
/// the overlay's span→element resolution.
const MODEL: &str = r#"package Demo {
    part duplicated;
    part duplicated;
    view def OverviewDef;
    view overview : OverviewDef {
        expose Demo;
    }
}
"#;

fn write_workspace() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sysml_diag_overlay_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create tempdir");
    fs::write(dir.join("Demo.sysml"), MODEL).expect("write model");
    dir
}

fn first_view_id(svc: &SysmlService) -> ElementId {
    let ws = svc.workspace_aware_graph().expect("workspace graph");
    ws.elements
        .values()
        .find(|e| matches!(format!("{:?}", e.kind).as_str(), "ViewUsage" | "ViewDefinition"))
        .map(|e| e.id.clone())
        .expect("model declares a view")
}

#[test]
fn diagnostic_overlay_succeeds_and_returns_elementid_keyed_shape() {
    let dir = write_workspace();
    let service = SysmlService::empty();
    service
        .open_context(OpenTarget::Folder(dir.clone()))
        .expect("open_context");

    // The model has real, published diagnostics (the duplicate-name warning).
    let diag_count: usize = service
        .loaded_uris()
        .into_iter()
        .filter(|u| u != "__workspace__")
        .filter_map(|u| service.diagnostics(&u).ok())
        .map(|ds| ds.len())
        .sum();
    assert!(diag_count > 0, "the workspace should carry at least one diagnostic");

    let view_id = first_view_id(&service);
    let expanded: HashSet<String> = HashSet::new();

    // Must NOT error (regression gate: the command previously called
    // `diagnostics("__workspace__")`, which errors on the sentinel).
    let overlay = service
        .diagram_diagnostic_overlay(&view_id, &expanded)
        .expect("diagnostic_overlay succeeds for a declared view over a workspace with diagnostics");

    // Shape: an ElementId-keyed `elements` object (sparse — may be empty when the
    // view doesn't render the diagnosed element; see the scope note above).
    assert!(
        overlay.get("elements").and_then(|e| e.as_object()).is_some(),
        "overlay carries an ElementId-keyed elements map"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn diagnostic_overlay_errors_on_unknown_view() {
    let service = SysmlService::empty();
    let ghost = ElementId::from_string("no-such-view");
    let expanded: HashSet<String> = HashSet::new();
    let err = service.diagram_diagnostic_overlay(&ghost, &expanded);
    assert!(err.is_err(), "an unknown view must fail hard, not return an empty overlay");
}
