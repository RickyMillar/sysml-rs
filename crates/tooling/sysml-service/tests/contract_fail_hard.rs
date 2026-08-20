//! Phase 0 — Fail-hard contract for `workspace_aware_graph` + `views_render`.
//!
//! Locks the post-P4 contract before the soft-fallback paths in
//! `SysmlService::workspace_aware_graph` (Q2) and `SysmlService::views_render`
//! (Q3) get rewritten to a single fail-hard branch. See
//!
//! The contract (ACTIVE — these tests run by default; the `views_render`
//! arm was enforced by RSC-6.5, Jun 18 2026):
//!
//!   workspace_aware_graph(uri) on a service without `open_context` →
//!     `Err(ServiceError::ElementNotFound("workspace not loaded; call open_context first"))`
//!
//!   views_render(uri, view_id, _) without a loaded workspace →
//!     `Err(_)` propagating the same shape (no silent per-file fallback —
//!     views are workspace-only; a per-file render would silently drop
//!     cross-file Expose targets).

use std::collections::HashSet;
use std::path::PathBuf;

use sysml_core::ElementId;
use sysml_project::discovery::OpenTarget;
use sysml_service::{ServiceError, SysmlService};

fn coffee_machine_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/book-examples/coffee-machine")
        .canonicalize()
        .expect("coffee-machine fixture must exist")
}

fn coffee_machine_file(name: &str) -> String {
    coffee_machine_root()
        .join(name)
        .to_string_lossy()
        .into_owned()
}

/// Substring the rewritten `workspace_aware_graph` error must contain. Pins
/// the post-P4 message body. If wording changes during P4, update here and
/// re-run.
const FAIL_HARD_MSG_HINT: &str = "workspace not loaded";

// ---------------------------------------------------------------------------
// workspace_aware_graph — fail-hard contract (Q2)
// ---------------------------------------------------------------------------

#[test]
fn workspace_aware_graph_fails_hard_pre_open() {
    // Scope-collapse W2: the accessor takes no uri — the only failure
    // mode left is "nothing was ever opened", and it must be a precise
    // error, not a silent per-file fallback.
    let svc = SysmlService::empty();

    let err = svc
        .workspace_aware_graph()
        .expect_err("empty service must error before any open");

    match &err {
        ServiceError::ElementNotFound(msg) => assert!(
            msg.contains(FAIL_HARD_MSG_HINT),
            "expected fail-hard message containing {:?}, got {:?}",
            FAIL_HARD_MSG_HINT,
            msg
        ),
        other => panic!(
            "expected ServiceError::ElementNotFound, got {:?}",
            other
        ),
    }
}

#[test]
fn workspace_aware_graph_succeeds_after_open_context() {
    let svc = SysmlService::empty();
    let ctx = svc
        .open_context(OpenTarget::Folder(coffee_machine_root()))
        .expect("coffee-machine open_context should succeed");
    assert!(
        ctx.loaded_uris.len() > 1,
        "open_context should load multiple files, got {:?}",
        ctx.loaded_uris
    );

    let ws_graph = svc
        .workspace_aware_graph()
        .expect("workspace graph must resolve after open_context");
    assert!(
        !ws_graph.elements.is_empty(),
        "workspace graph must have elements after open_context"
    );
}

// ---------------------------------------------------------------------------
// check_constraints — file_element_ids fail-hard contract (P1)
// ---------------------------------------------------------------------------

#[test]
fn contract_check_constraints_propagates_file_element_ids_failure() {
    let svc = SysmlService::empty();
    // No workspace loaded — `workspace_aware_graph` will Err. Today
    // `check_constraints` silently re-scopes this to a workspace-wide
    // sweep (or returns Ok(vec![])) via the file_element_ids soft
    // fallback. Post-P1 it must propagate the workspace-load error.
    let bogus_uri = "nonexistent-file.sysml";

    let result = svc.check_constraints(bogus_uri, &[]);

    let err = result.expect_err(
        "check_constraints on an empty service must error, not silently \
         re-scope to a workspace-wide sweep or return Ok(vec![])",
    );
    let msg = format!("{:?}", err);
    assert!(
        msg.to_lowercase().contains("workspace"),
        "expected error mentioning {:?}, got {:?}",
        FAIL_HARD_MSG_HINT,
        msg
    );
}

// ---------------------------------------------------------------------------
// views_render — fail-hard contract (Q3)
// ---------------------------------------------------------------------------

#[test]
fn views_render_fails_hard_on_empty_service() {
    let svc = SysmlService::empty();
    // Synthetic view-usage id; service has no model loaded so the only
    // sensible behavior is fail-hard. Currently this errors via
    // `require_graph(uri)`; post-Q3 it should error via the rewritten
    // `workspace_aware_graph` (single failure mode, single message).
    let view_id = ElementId::from_string("synthetic-nonexistent-view-id");
    let expanded: HashSet<String> = HashSet::new();

    let result = svc.views_render("file:///does/not/exist.sysml", &view_id, &expanded);
    assert!(
        result.is_err(),
        "views_render on an empty service must error, got Ok"
    );
}

#[test]
fn views_render_succeeds_after_open_context() {
    let svc = SysmlService::empty();
    svc.open_context(OpenTarget::Folder(coffee_machine_root()))
        .expect("coffee-machine open_context should succeed");

    // Pick the first user-authored ViewUsage from the merged graph. The
    // coffee-machine fixture's views.sysml declares several (e.g.
    // `structuralOverview`).
    let ws_graph = svc
        .workspace_aware_graph()
        .expect("workspace graph must resolve");
    let view_id = ws_graph
        .elements
        .values()
        .find(|e| {
            // Match SysML v2 view kinds — either declaration form.
            matches!(
                format!("{:?}", e.kind).as_str(),
                "ViewUsage" | "ViewDefinition"
            )
        })
        .map(|e| e.id.clone())
        .expect("coffee-machine fixture must declare at least one view");

    let expanded: HashSet<String> = HashSet::new();
    let rendered = svc
        .views_render(&coffee_machine_file("views.sysml"), &view_id, &expanded)
        .expect("views_render of a real view id must succeed after open_context");
    // SModel JSON payload — non-null, contains a top-level node id.
    assert!(
        rendered.is_object(),
        "rendered view must be a JSON object, got {:?}",
        rendered
    );
}
