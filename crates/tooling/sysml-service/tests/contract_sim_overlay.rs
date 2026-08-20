//! Contract: `sysml.diagram.sim_overlay` (Bucket 1.8, re-keyed in Bucket 2 F2)
//! returns the per-tick simulation overlay for a live session — active-element
//! highlights + a time-series channel directory — joined to a **declared view's**
//! scene by `ElementId`, and fails hard on an unknown session.
//!
//! Post-F2 the overlay joins to a declared view (ViewUsage / ViewDefinition),
//! passed as `view_usage_id` — the SAME id the caller passes to
//! `sysml.diagram.viewmodel`, so the overlay element ids align with the scene.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use serde_json::json;
use sysml_core::ElementId;
use sysml_project::discovery::OpenTarget;
use sysml_service::{execute_command, SysmlService};

/// A self-contained workspace: a runnable two-state SM (mirrors ValveGating's
/// ValveSM) plus a declared view exposing the package, so the overlay has a
/// framed scene to join against.
const MODEL: &str = r#"package Demo {
    state def ValveSM {
        state open;
        state closed;
        entry; then open;
        transition open_to_closed
            first open
            accept when true
            then closed;
    }
    view def OverviewDef;
    view overview : OverviewDef {
        expose Demo;
    }
}
"#;

fn write_workspace() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sysml_sim_overlay_{}", std::process::id()));
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
fn sim_overlay_returns_tick_and_channels_for_a_declared_view() {
    let dir = write_workspace();
    let service = SysmlService::empty();
    service
        .open_context(OpenTarget::Folder(dir.clone()))
        .expect("open_context");

    let uri = service
        .loaded_uris()
        .into_iter()
        .find(|u| u.contains("Demo"))
        .expect("Demo URI");

    // Start + step the SM so a snapshot exists.
    let start = execute_command(
        &service,
        "sysml.simulate.start",
        json!({ "uri": uri, "sm_name": "ValveSM" }),
    )
    .expect("simulate.start");
    let session_id = start.as_array().unwrap()[0].as_str().unwrap().to_owned();
    execute_command(&service, "sysml.simulate.step", json!({ "session_key": session_id }))
        .expect("simulate.step");

    let view_id = first_view_id(&service);
    let expanded: HashSet<String> = HashSet::new();
    let overlay = service
        .diagram_sim_overlay(&session_id, &view_id, &expanded)
        .expect("sim_overlay succeeds for a declared view");

    // Shape: tick (number), elements (object), channels (array).
    assert!(overlay.get("tick").and_then(|t| t.as_u64()).is_some(), "overlay carries a tick");
    assert!(
        overlay.get("elements").and_then(|e| e.as_object()).is_some(),
        "overlay carries an elements map"
    );
    assert!(
        overlay.get("channels").and_then(|c| c.as_array()).is_some(),
        "overlay carries channels"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn sim_overlay_errors_on_unknown_session() {
    let service = SysmlService::empty();
    let view_id = ElementId::from_string("any-view");
    let expanded: HashSet<String> = HashSet::new();
    let err = service.diagram_sim_overlay("no-such-session", &view_id, &expanded);
    assert!(err.is_err(), "unknown session must fail hard, not return an empty overlay");
}
