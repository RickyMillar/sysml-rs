//! Same-root reload contract — `sysml.load_workspace` on an already-loaded
//! root must pick up disk edits (the FE reload button's demo loop:
//! edit file → reload → suspect flags).
//!
//! Live-observed staleness (2026-07-16): with the app's Source panel open,
//! the focused file is an LSP editor overlay, and `open_context` skips
//! disk reads for overlaid files — so a same-root reload kept the old
//! text forever. The old workaround, `sysml.workspace.refresh` + re-load,
//! transiently emptied `__workspace__` (host nuke without content reload).
//!
//! These tests pin the reload contract at the service layer:
//!  - pure path (no overlays): reload sees disk edits
//!  - overlay path: reload is disk-authoritative for load_workspace
//!  - no empty-workspace window inside a reload

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use sysml_service::SysmlService;
use tempfile::TempDir;

const OLD_MODEL: &str = "package Reqs {\n    part def OldWidget;\n}\n";
const NEW_MODEL: &str = "package Reqs {\n    part def NewWidget;\n}\n";

const DOC_OLD: &str =
    "package Reqs {\n    requirement def R1 {\n        doc /* the old statement */\n    }\n}\n";
const DOC_NEW: &str =
    "package Reqs {\n    requirement def R1 {\n        doc /* the NEW statement */\n    }\n}\n";

fn workspace_with(content: &str) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("model.sysml");
    fs::write(&file, content).unwrap();
    (dir, file)
}

fn workspace_has_element(service: &SysmlService, name: &str) -> bool {
    let graph = service.workspace_aware_graph().expect("workspace graph");
    graph
        .elements
        .values()
        .any(|e| e.name.as_deref() == Some(name))
}

/// Pure service path: no editor overlays anywhere. A second
/// `load_workspace` of the same root must re-read changed files from disk.
#[test]
fn same_root_reload_picks_up_disk_edits() {
    let (dir, file) = workspace_with(OLD_MODEL);
    let service = SysmlService::empty();

    service.load_workspace(dir.path()).unwrap();
    assert!(
        workspace_has_element(&service, "OldWidget"),
        "initial load should see OldWidget"
    );

    fs::write(&file, NEW_MODEL).unwrap();
    service.load_workspace(dir.path()).unwrap();

    assert!(
        workspace_has_element(&service, "NewWidget"),
        "same-root reload must pick up the disk edit"
    );
    assert!(
        !workspace_has_element(&service, "OldWidget"),
        "stale pre-edit text must be gone after reload"
    );
}

/// Editor-overlay path — the live-observed bug. The app's Source panel
/// did_opens the focused file over the LSP websocket, marking it an
/// overlay. `sysml.load_workspace` is an explicit "load this root from
/// disk" command: disk is authoritative for it, overlay or not.
#[test]
fn same_root_reload_is_disk_authoritative_over_overlays() {
    let (dir, file) = workspace_with(OLD_MODEL);
    let service = SysmlService::empty();

    service.load_workspace(dir.path()).unwrap();

    // Simulate the LSP did_open: buffer content + overlay flag, exactly
    // what sysml-lsp-server does under one host lock.
    let uri = file.to_string_lossy().to_string();
    {
        let mut host = service.host_arc().lock().unwrap();
        host.set_overlay(&uri);
    }

    // External edit on disk while the editor overlay is live.
    fs::write(&file, NEW_MODEL).unwrap();
    service.load_workspace(dir.path()).unwrap();

    assert!(
        workspace_has_element(&service, "NewWidget"),
        "reload must be disk-authoritative even for overlaid files"
    );
    assert!(
        !workspace_has_element(&service, "OldWidget"),
        "overlay must not pin stale pre-edit text across a reload"
    );
}

/// A DOC-ONLY edit must propagate through reload just like a structural
/// one. Pins the live-observed 2026-07-16 staleness: the salsa
/// fingerprint wrappers backdated re-parses whose graphs differed only
/// in doc text / property values (`ModelGraph::compute_fingerprint`
/// hashed only name+kind), so requirement rows kept pre-edit statements
/// across reloads until a structural edit happened to land.
#[test]
fn same_root_reload_picks_up_doc_only_edits() {
    let (dir, file) = workspace_with(DOC_OLD);
    let service = SysmlService::empty();

    service.load_workspace(dir.path()).unwrap();
    let doc_text = |service: &SysmlService| -> String {
        let graph = service.workspace_aware_graph().expect("workspace graph");
        graph
            .elements
            .values()
            .filter_map(|e| e.props.get("body"))
            .map(|v| format!("{v:?}"))
            .chain(
                graph
                    .elements
                    .values()
                    .flat_map(|e| e.props.iter())
                    .map(|(k, v)| format!("{k}={v:?}")),
            )
            .collect::<Vec<_>>()
            .join("\n")
    };
    let before = doc_text(&service);
    assert!(
        before.contains("the old statement"),
        "initial load must surface the doc text; got:\n{before}"
    );

    fs::write(&file, DOC_NEW).unwrap();
    service.load_workspace(dir.path()).unwrap();

    let after = doc_text(&service);
    assert!(
        after.contains("the NEW statement"),
        "doc-only disk edit must survive a same-root reload; still serving:\n{after}"
    );
}

/// After a disk-authoritative reload the overlay flag itself is gone —
/// the next implicit indexer pass tracks disk again instead of
/// re-freezing the stale buffer.
#[test]
fn reload_clears_overlay_flags() {
    let (dir, file) = workspace_with(OLD_MODEL);
    let service = SysmlService::empty();
    service.load_workspace(dir.path()).unwrap();

    let uri = file.to_string_lossy().to_string();
    {
        let mut host = service.host_arc().lock().unwrap();
        host.set_overlay(&uri);
        assert!(host.has_overlay(&uri), "precondition: overlay set");
    }

    service.load_workspace(dir.path()).unwrap();
    let host = service.host_arc().lock().unwrap();
    assert!(
        !host.has_overlay(&uri),
        "load_workspace must clear editor-overlay flags (disk is truth)"
    );
}

/// The implicit `open_context(Folder)` path — the LSP background
/// indexer's rescan — keeps the overlay-preserving rule. Only
/// `load_workspace` is disk-authoritative; the two semantics share one
/// call site distinguished by `OverlayPolicy` (steward red line 1).
#[test]
fn indexer_rescan_preserves_overlays_where_reload_does_not() {
    use sysml_project::discovery::OpenTarget;

    let (dir, file) = workspace_with(OLD_MODEL);
    let service = SysmlService::empty();
    service.load_workspace(dir.path()).unwrap();

    // Simulate did_open with an UNSAVED buffer that differs from disk:
    // buffer content + overlay flag under one host lock, like the LSP.
    let uri = file.to_string_lossy().to_string();
    const BUFFER: &str = "package Reqs {\n    part def BufferWidget;\n}\n";
    {
        let mut host = service.host_arc().lock().unwrap();
        host.set_file_content(&uri, BUFFER.to_owned());
        host.set_overlay(&uri);
    }

    // Implicit rescan (OverlayPolicy::Preserve default): the open
    // buffer stays authoritative.
    service
        .open_context(OpenTarget::Folder(dir.path().to_path_buf()))
        .unwrap();
    assert!(
        workspace_has_element(&service, "BufferWidget"),
        "indexer rescan must NOT clobber an open editor buffer from disk"
    );

    // Explicit reload (DiskAuthoritative): disk wins, overlay cleared.
    service.load_workspace(dir.path()).unwrap();
    assert!(
        workspace_has_element(&service, "OldWidget"),
        "load_workspace must re-assert disk content over the buffer"
    );
    assert!(
        !workspace_has_element(&service, "BufferWidget"),
        "stale buffer text must be gone after an explicit reload"
    );
}

/// Files deleted on disk drop out of the workspace on reload — no
/// ghost elements from a file discovery no longer lists.
#[test]
fn reload_drops_files_deleted_on_disk() {
    let dir = TempDir::new().unwrap();
    let keep = dir.path().join("keep.sysml");
    let doomed = dir.path().join("doomed.sysml");
    fs::write(&keep, "package Keep {\n    part def Keeper;\n}\n").unwrap();
    fs::write(&doomed, "package Doomed {\n    part def Ghost;\n}\n").unwrap();

    let service = SysmlService::empty();
    service.load_workspace(dir.path()).unwrap();
    assert!(workspace_has_element(&service, "Ghost"), "precondition");

    fs::remove_file(&doomed).unwrap();
    let result = service.load_workspace(dir.path()).unwrap();

    assert!(
        workspace_has_element(&service, "Keeper"),
        "surviving file still loaded"
    );
    assert!(
        !workspace_has_element(&service, "Ghost"),
        "deleted file must not leave ghost elements in the workspace"
    );
    assert!(
        !result
            .loaded_uris
            .iter()
            .any(|u| u.ends_with("doomed.sysml")),
        "deleted file must not appear in loaded_uris"
    );
}

/// Same-root reload keeps in-root `FileId` identity (the canonical-form
/// scope prefix retains in-root files instead of wiping them), so salsa
/// can backdate unchanged files instead of re-elaborating from scratch.
#[test]
fn same_root_reload_retains_file_identity() {
    let (dir, file) = workspace_with(OLD_MODEL);
    let service = SysmlService::empty();
    service.load_workspace(dir.path()).unwrap();

    let uri = file.to_string_lossy().to_string();
    let id_before = {
        let host = service.host_arc().lock().unwrap();
        host.file_id(&uri).expect("file tracked after load")
    };

    service.load_workspace(dir.path()).unwrap();
    let id_after = {
        let host = service.host_arc().lock().unwrap();
        host.file_id(&uri).expect("file tracked after reload")
    };
    assert_eq!(
        id_before, id_after,
        "same-root reload must retain FileId identity for incrementality"
    );
}

/// A reload never exposes an empty workspace to concurrent readers:
/// after the reload call returns, the graph is fully populated. (The
/// old refresh+reload workaround had a window where `__workspace__`
/// held zero elements.)
#[test]
fn reload_leaves_no_empty_workspace_window() {
    let (dir, _file) = workspace_with(OLD_MODEL);
    let service = SysmlService::empty();

    service.load_workspace(dir.path()).unwrap();
    let before = service
        .workspace_aware_graph()
        .expect("graph before reload")
        .elements
        .len();
    assert!(before > 0, "workspace should be non-empty after load");

    service.load_workspace(dir.path()).unwrap();
    let after = service
        .workspace_aware_graph()
        .expect("graph after reload")
        .elements
        .len();
    assert_eq!(
        before, after,
        "same-root reload of unchanged content must not shrink the workspace"
    );
}
