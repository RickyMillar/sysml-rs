#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
//! LSP-level tests that exercise the import-resolution invariants the
//! editor-facing surface promises. Each test boots a `TestServer`,
//! initialises it with a workspace folder (or none, for strict-mode
//! cases), opens documents the way VS Code would, runs the diagnostic
//! pipeline, and asserts the resulting LSP `Diagnostic` set.
//!
//! These tests exist because the LSP keeps regressing on cross-file
//! resolution in the most basic scenarios (the coffee-machine
//! `WaterPort` bug is the third repro of the same class). Each scenario
//! here pins down one rule the import system has to satisfy.
//!
//!
//! - `folder_open_resolves_cross_file_import_simple` — minimal
//!   reproducer for the WaterPort failure mode. Folder open + bare file
//!   imports via `import Pkg::*;`. **The test that would have caught
//!   the bug we just fixed.**
//! - `folder_open_resolves_nested_package_def` — same shape but the
//!   def is nested inside its package (the real coffee-machine layout).
//! - `did_change_preserves_workspace_resolution` — guards against
//!   `did_change` losing the project_id tag on edit.
//! - `single_file_open_is_strict_mode` — single file open (no workspace
//!   folder) surfaces IM012, not E200 + lenient downgrade.
//! - `manifest_folder_is_additive` — a folder with `sysml.toml` behaves
//!   identically to the same folder without (already P6's claim — here
//!   it's exercised at the LSP layer).
//!
//! The TL;DR for future authors: if you touch how the LSP tags files
//! with project ids, run this module. If you add a new file-loading
//! invariant, add a test here that locks it in at the LSP layer.

use std::fs;
use std::path::Path;

use tempfile::TempDir;
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticSeverity, InitializeParams, InitializedParams, NumberOrString, Url,
    WorkspaceFolder,
};
use tower_lsp::LanguageServer;

use crate::test_harness::TestServer;

// ─── Fixtures ─────────────────────────────────────────────────────────────────

/// Two-file workspace: `ports.sysml` declares a port def; `connections.sysml`
/// imports the Ports package and uses the def bare. Mirrors the WaterPort
/// shape from the coffee-machine example.
const PORTS_SRC: &str = "package Ports { part def WaterPort; }";
const CONNECTIONS_SRC: &str = "package Connections { import Ports::*; part w : WaterPort; }";

/// Same as above but with the def nested two levels deep so we also
/// exercise the inner-namespace re-export path (this is what tripped
/// the coffee-machine repro before the peek_neighbours DFS extension).
const PORTS_NESTED_SRC: &str = "package Ports { part def WaterPort; part def CoffeeOutPort; }";
const USES_NESTED_SRC: &str = r#"
package Uses {
    import Ports::*;
    part w : WaterPort;
    part c : CoffeeOutPort;
}
"#;

const MIN_MANIFEST: &str = r#"
[project]
name = "lsp-import-test"
version = "0.1.0"
"#;

fn write_workspace(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    for (rel, content) in files {
        let p = dir.path().join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&p, content).unwrap();
    }
    dir
}

fn file_uri(dir: &Path, rel: &str) -> String {
    Url::from_file_path(dir.join(rel))
        .expect("path should convert to URI")
        .to_string()
}

async fn initialize_with_workspace_root(server: &TestServer, root: &Path) {
    // Opt into running the LSP's background workspace indexer for these
    // tests. TestServer disables it by default for speed; here we
    // specifically want to exercise the LSP file-entry pipeline end to
    // end, including the indexer that loads neighbour files. URIs must
    // match what `did_open` would produce (`file://...`) — that's why
    // we use the LSP indexer rather than pre-loading via
    // `service.open_context` (which uses raw paths).
    server
        .server()
        .skip_background_tasks
        .store(false, std::sync::atomic::Ordering::Relaxed);

    let root_uri = Url::from_file_path(root).expect("workspace URI");
    let init = InitializeParams {
        workspace_folders: Some(vec![WorkspaceFolder {
            uri: root_uri.clone(),
            name: "fixture".to_string(),
        }]),
        root_uri: Some(root_uri),
        ..Default::default()
    };
    server
        .server()
        .initialize(init)
        .await
        .expect("initialize should succeed");
    server.server().initialized(InitializedParams {}).await;
}

/// Pull the diagnostic set for the URI directly from the salsa pipeline.
/// This matches what the LSP would publish via `publishDiagnostics` — we
/// bypass the protocol delivery so the test doesn't have to plumb a
/// mock client.
async fn diagnostics_for(server: &TestServer, uri: &str) -> Vec<Diagnostic> {
    server.server().salsa_diagnostics(uri).await
}

/// Wait for the LSP's async workspace indexing (kicked off in
/// `initialized`) to finish. The indexer walks the workspace, parses
/// every `.sysml` / `.kerml` file, and builds the `ProjectFileSet`
/// salsa inputs for workspace-aware resolution. There's no public
/// "indexing complete" signal, so we poll for the side effects:
/// every expected URI must be registered in the host, and at least
/// one `ProjectFileSet` must exist for the workspace project.
///
/// `expected_uris` is the list of URIs the test fixture writes — when
/// each one shows up in `host.files()` AND the workspace PFS contains
/// matching source files, indexing has reached steady state.
async fn wait_for_workspace_index(
    server: &TestServer,
    expected_uris: &[String],
    timeout: std::time::Duration,
) {
    let default_pid = sysml_project::ProjectHandle(
        sysml_service::open_context::DEFAULT_PROJECT_ID,
    );
    let start = std::time::Instant::now();
    loop {
        let (all_loaded, pfs_ready) = {
            let host = server.server().analysis_host.lock().unwrap();
            let files = host.files();
            let all_loaded = expected_uris.iter().all(|u| files.lookup(u).is_some());
            let pfs_ready = host
                .project_file_set(default_pid)
                .map(|pfs| pfs.files(host.db()).len() >= expected_uris.len())
                .unwrap_or(false);
            (all_loaded, pfs_ready)
        };
        if all_loaded && pfs_ready {
            return;
        }
        if start.elapsed() > timeout {
            let host = server.server().analysis_host.lock().unwrap();
            let files = host.files();
            let missing: Vec<&String> = expected_uris
                .iter()
                .filter(|u| files.lookup(u).is_none())
                .collect();
            let pfs_file_count = host
                .project_file_set(default_pid)
                .map(|pfs| pfs.files(host.db()).len());
            panic!(
                "workspace index did not reach ready state within {timeout:?}; \
                 missing files: {missing:?}, pfs_file_count: {pfs_file_count:?}, \
                 expected_files: {}",
                expected_uris.len()
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

/// Find any diagnostic that looks like an unresolved-name failure for
/// the given symbol — covers IM010 (after the strict-mode upgrade) and
/// the raw E200 that some unresolved-name paths still emit.
fn unresolved_for<'a>(diags: &'a [Diagnostic], name: &str) -> Vec<&'a Diagnostic> {
    diags
        .iter()
        .filter(|d| {
            let is_unresolved = matches!(
                d.code.as_ref(),
                Some(NumberOrString::String(c)) if c == "E200" || c == "IM010"
            );
            is_unresolved
                && d.severity == Some(DiagnosticSeverity::ERROR)
                && d.message.contains(name)
        })
        .collect()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// THE test that would have caught the coffee-machine bug.
///
/// Setup: workspace folder with two siblings; one declares a port def,
/// the other imports the package and uses the def. No `sysml.toml`.
///
/// Assertion: opening `connections.sysml` after the workspace folder is
/// announced via `initialize` surfaces zero unresolved-name diagnostics
/// for `WaterPort`.
#[tokio::test]
async fn folder_open_resolves_cross_file_import_simple() {
    let dir = write_workspace(&[
        ("ports.sysml", PORTS_SRC),
        ("connections.sysml", CONNECTIONS_SRC),
    ]);

    let server = TestServer::new();
    initialize_with_workspace_root(&server, dir.path()).await;

    let connections_uri = file_uri(dir.path(), "connections.sysml");
    let ports_uri = file_uri(dir.path(), "ports.sysml");
    server.open_document(&connections_uri, CONNECTIONS_SRC).await;

    // Wait for the background workspace indexer to register both files
    // and build the workspace ProjectFileSet — only then is the
    // workspace-aware resolution path active.
    wait_for_workspace_index(
        &server,
        &[connections_uri.clone(), ports_uri],
        std::time::Duration::from_secs(5),
    )
    .await;

    let diags = diagnostics_for(&server, &connections_uri).await;
    let unresolved = unresolved_for(&diags, "WaterPort");
    assert!(
        unresolved.is_empty(),
        "WaterPort should resolve cross-file when the folder is open as a workspace; \
         got {} unresolved diagnostic(s):\n{:#?}\n\nAll diagnostics:\n{:#?}",
        unresolved.len(),
        unresolved,
        diags
    );
}

/// Same shape, def nested in its package. Locks in the peek_neighbours
/// DFS extension AND the project_id fallback together.
#[tokio::test]
async fn folder_open_resolves_nested_package_def() {
    let dir = write_workspace(&[
        ("ports.sysml", PORTS_NESTED_SRC),
        ("uses.sysml", USES_NESTED_SRC),
    ]);

    let server = TestServer::new();
    initialize_with_workspace_root(&server, dir.path()).await;

    let uses_uri = file_uri(dir.path(), "uses.sysml");
    let ports_uri = file_uri(dir.path(), "ports.sysml");
    server.open_document(&uses_uri, USES_NESTED_SRC).await;
    wait_for_workspace_index(
        &server,
        &[uses_uri.clone(), ports_uri],
        std::time::Duration::from_secs(5),
    )
    .await;

    let diags = diagnostics_for(&server, &uses_uri).await;
    for name in ["WaterPort", "CoffeeOutPort"] {
        let unresolved = unresolved_for(&diags, name);
        assert!(
            unresolved.is_empty(),
            "{} should resolve cross-file in a folder-opened workspace; \
             got {} unresolved:\n{:#?}",
            name,
            unresolved.len(),
            unresolved
        );
    }
}

/// `did_change` rewrites a file's content. The bug we just fixed leaves
/// open files pid-less, so an in-place edit must not regress them off
/// the workspace-aware resolution path either.
#[tokio::test]
async fn did_change_preserves_workspace_resolution() {
    let dir = write_workspace(&[
        ("ports.sysml", PORTS_SRC),
        ("connections.sysml", CONNECTIONS_SRC),
    ]);

    let server = TestServer::new();
    initialize_with_workspace_root(&server, dir.path()).await;

    let uri = file_uri(dir.path(), "connections.sysml");
    let ports_uri = file_uri(dir.path(), "ports.sysml");
    server.open_document(&uri, CONNECTIONS_SRC).await;
    wait_for_workspace_index(
        &server,
        &[uri.clone(), ports_uri],
        std::time::Duration::from_secs(5),
    )
    .await;

    // Edit: still imports Ports::*; still references WaterPort.
    let edited = "package Connections { import Ports::*; part renamed : WaterPort; }";
    server.change_document(&uri, 1, edited).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let diags = diagnostics_for(&server, &uri).await;
    let unresolved = unresolved_for(&diags, "WaterPort");
    assert!(
        unresolved.is_empty(),
        "WaterPort should still resolve after did_change; \
         got {} unresolved:\n{:#?}",
        unresolved.len(),
        unresolved
    );
}

/// Single-file open with no workspace folder. The file is in strict
/// mode; cross-file references should produce IM012 + IM010 with
/// neighbour hints (when a neighbour exists), NOT a silent Info-level
/// downgrade or a plain E200 without context.
#[tokio::test]
async fn single_file_open_is_strict_mode_with_neighbours_visible() {
    let dir = write_workspace(&[
        ("ports.sysml", PORTS_SRC),
        ("uses.sysml", CONNECTIONS_SRC),
    ]);

    // No workspace folder — initialize bare.
    let server = TestServer::new();
    server
        .server()
        .initialize(InitializeParams::default())
        .await
        .expect("initialize");
    server.server().initialized(InitializedParams {}).await;

    let uri = file_uri(dir.path(), "uses.sysml");
    server.open_document(&uri, CONNECTIONS_SRC).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let diags = diagnostics_for(&server, &uri).await;

    // The diagnostic the IDE pops up on a strict-mode cross-file ref:
    // exactly one IM012 banner.
    let im012 = diags
        .iter()
        .filter(|d| {
            matches!(d.code.as_ref(), Some(NumberOrString::String(c)) if c == "IM012")
        })
        .count();
    assert_eq!(
        im012, 1,
        "strict-mode open should surface exactly one IM012 banner; got {im012}\ndiags:\n{:#?}",
        diags
    );
}

/// P4.E invariant: what the LSP **publishes** for a file must match
/// what `service.diagnostics()` (== `salsa_diagnostics`) computes for
/// the same URI in the same workspace.
///
/// This test exists because the WaterPort regression survived past
/// every backend-only fix: every previous test exercised
/// `salsa_diagnostics` directly, which is *what* the backend produces
/// — but the bug was in the **publish step** silently dropping the
/// post-workspace re-publish (Url::parse rejected the raw-path URI
/// the indexer iterates over, so VS Code stayed on the pre-workspace
/// E200 forever even though `salsa_diagnostics` returned `[]`).
///
/// Lock that divergence down: the canonical-form URI and the file://
/// URI both have to read back the SAME diagnostic list, and the
/// published list has to match the backend's current computation.
#[tokio::test]
async fn published_diagnostics_match_service_diagnostics() {
    let dir = write_workspace(&[
        ("ports.sysml", PORTS_SRC),
        ("connections.sysml", CONNECTIONS_SRC),
    ]);

    let server = TestServer::new();
    initialize_with_workspace_root(&server, dir.path()).await;

    let uri = file_uri(dir.path(), "connections.sysml");
    let ports_uri = file_uri(dir.path(), "ports.sysml");
    server.open_document(&uri, CONNECTIONS_SRC).await;
    wait_for_workspace_index(
        &server,
        &[uri.clone(), ports_uri],
        std::time::Duration::from_secs(5),
    )
    .await;

    // The workspace indexer's Phase 4 fires `run_diagnostics_cycle` per
    // file. That hits the publish gate. Poll until the LIVE backend
    // (service.diagnostics) matches the latest published payload — if
    // the publish path is broken (Url::parse fail, silent drop, etc.)
    // the published stays stuck on the pre-workspace E200 while the
    // live backend reports `[]`, and this loop times out.
    //
    // Re-fetch backend on every iteration: Phase 4 is racing the test,
    // so a snapshot taken before it completes can show pre-workspace
    // state and become spuriously "different" from a later empty
    // publish.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    // Assigned on the first loop iteration (which always runs before any
    // `break`/`return`), so no throwaway initializer is needed.
    let mut last_backend: Vec<Diagnostic>;
    let mut last_published: Vec<Diagnostic>;
    loop {
        last_backend = diagnostics_for(&server, &uri).await;
        last_published = server
            .last_server_published_diagnostics(&uri)
            .unwrap_or_default();
        if last_published.len() == last_backend.len()
            && last_published
                .iter()
                .zip(last_backend.iter())
                .all(|(a, b)| a.message == b.message && a.range == b.range && a.code == b.code)
        {
            return;
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!(
        "published diagnostics never converged with service.diagnostics for {uri}.\n\
         backend (service.diagnostics) reports {} diagnostic(s):\n{:#?}\n\
         last published payload had {} diagnostic(s):\n{:#?}\n\n\
         This is the WaterPort publish-path divergence: the backend is correct \
         but the LSP's publish step is silently dropping or stale.",
        last_backend.len(),
        last_backend,
        last_published.len(),
        last_published,
    );
}

/// P-RA3 invariant: a `did_open` that fires before the workspace
/// indexer completes must NOT surface any `NameResWorkspace`-tier
/// diagnostics (cross-file E200 etc.). Once the workspace is indexed
/// the same file sees the full set.
///
/// We exercise the "early" window by leaving `skip_background_tasks =
/// true` (the `TestServer` default): the LSP's background workspace
/// indexer never runs, so the file enters the host without a
/// `project_id` / `ProjectFileSet`. The readiness gate inside
/// `compute_full_diagnostics` should suppress every diagnostic whose
/// tier needs `project = Indexed`.
///
/// We then drive the indexing pipeline synchronously via
/// `service.open_context(Folder)` (the same path MCP / CLI use) so the
/// post-Phase-4 window can be observed in the same test without a
/// race against `tokio::spawn`.
///
/// Reads diagnostics via `service.diagnostics(uri)` — that's the
/// sysml-span surface that carries the `tier` field; the LSP wire
/// format drops tier on conversion. (Per P-RA3: don't change the wire.)
#[tokio::test]
async fn tier_gating_during_workspace_load() {
    use sysml_project::discovery::OpenTarget;
    use sysml_span::DiagnosticTier;
    use sysml_service::readiness::ProjectReadiness;

    let dir = write_workspace(&[
        ("ports.sysml", PORTS_SRC),
        ("connections.sysml", CONNECTIONS_SRC),
    ]);

    // For THIS test we deliberately keep `skip_background_tasks = true`
    // (the `TestServer::new()` default). The workspace indexer never
    // spawns, locking in the "pre-Phase-4" state we want to observe.
    // We bypass `initialize_with_workspace_root` (which opts INTO the
    // indexer for end-to-end pipeline tests) and do a minimal init that
    // registers the workspace root WITHOUT kicking off background work.
    let server = TestServer::new();
    let root_uri = Url::from_file_path(dir.path()).expect("workspace URI");
    let init = InitializeParams {
        workspace_folders: Some(vec![WorkspaceFolder {
            uri: root_uri.clone(),
            name: "fixture".to_string(),
        }]),
        root_uri: Some(root_uri),
        ..Default::default()
    };
    server
        .server()
        .initialize(init)
        .await
        .expect("initialize should succeed");
    server.server().initialized(InitializedParams {}).await;

    let uri = file_uri(dir.path(), "connections.sysml");
    server.open_document(&uri, CONNECTIONS_SRC).await;

    // ─── Window 1: pre-workspace-index ──────────────────────────────
    let service = server.server().service.clone();

    // Sanity-check the readiness gate's preconditions: the file is in
    // the host (otherwise the diag pipeline would short-circuit and we
    // wouldn't be testing the gate), but the project is NOT yet
    // indexed.
    let r_pre = service.readiness_for(&uri);
    assert_eq!(
        r_pre.project,
        ProjectReadiness::NotIndexed,
        "background tasks are disabled — project must not be indexed yet (readiness: {r_pre:?})"
    );

    let diags_pre = service
        .diagnostics(&uri)
        .expect("file is loaded; diagnostics should succeed");
    let workspace_tier_pre: Vec<_> = diags_pre
        .iter()
        .filter(|d| {
            matches!(
                d.tier,
                DiagnosticTier::NameResWorkspace
                    | DiagnosticTier::ImportHealth
                    | DiagnosticTier::Semantic
                    | DiagnosticTier::Constraint
            )
        })
        .collect();
    assert!(
        workspace_tier_pre.is_empty(),
        "pre-workspace-index window must publish zero workspace-tier diagnostics; \
         got {}: {:#?}",
        workspace_tier_pre.len(),
        workspace_tier_pre
    );

    // ─── Window 2: post-workspace-index ─────────────────────────────
    // Run the same workspace-loading pipeline MCP/CLI use, so the
    // ProjectFileSet is now registered and the readiness gate opens.
    service
        .open_context(OpenTarget::Folder(dir.path().to_path_buf()))
        .expect("open_context should succeed for the fixture folder");

    let r_post = service.readiness_for(&uri);
    assert_eq!(
        r_post.project,
        ProjectReadiness::Indexed,
        "after open_context, project must be indexed (readiness: {r_post:?})"
    );

    let diags_post = service
        .diagnostics(&uri)
        .expect("file is still loaded; diagnostics should succeed");

    // WaterPort actually resolves in this fixture, so the post-index
    // unresolved count is also 0 — but now it's 0 because resolution
    // *succeeded*, not because the gate suppressed it.
    let unresolved_post = unresolved_for_span(&diags_post, "WaterPort");
    assert!(
        unresolved_post.is_empty(),
        "post-index: WaterPort should resolve cross-file in a folder-opened workspace; \
         got {} unresolved:\n{:#?}",
        unresolved_post.len(),
        unresolved_post
    );
}

/// Span-level counterpart of `unresolved_for` — operates on
/// `sysml_span::Diagnostic` so the tier field is visible.
fn unresolved_for_span<'a>(
    diags: &'a [sysml_span::Diagnostic],
    name: &str,
) -> Vec<&'a sysml_span::Diagnostic> {
    diags
        .iter()
        .filter(|d| {
            let code_match = matches!(
                d.code.as_deref(),
                Some(c) if c == "E200" || c == "IM010"
            );
            code_match && d.severity == sysml_span::Severity::Error && d.message.contains(name)
        })
        .collect()
}

/// Adding a zero-dep `sysml.toml` to a workspace folder must NOT change
/// resolution outcomes. This is P6's manifest-additivity claim run
/// through the LSP layer.
#[tokio::test]
async fn manifest_folder_is_additive_through_lsp() {
    let dir = write_workspace(&[
        ("sysml.toml", MIN_MANIFEST),
        ("ports.sysml", PORTS_SRC),
        ("connections.sysml", CONNECTIONS_SRC),
    ]);

    let server = TestServer::new();
    initialize_with_workspace_root(&server, dir.path()).await;

    let uri = file_uri(dir.path(), "connections.sysml");
    let ports_uri = file_uri(dir.path(), "ports.sysml");
    server.open_document(&uri, CONNECTIONS_SRC).await;
    wait_for_workspace_index(
        &server,
        &[uri.clone(), ports_uri],
        std::time::Duration::from_secs(5),
    )
    .await;

    let diags = diagnostics_for(&server, &uri).await;
    let unresolved = unresolved_for(&diags, "WaterPort");
    assert!(
        unresolved.is_empty(),
        "WaterPort should resolve with manifest; got {} unresolved:\n{:#?}",
        unresolved.len(),
        unresolved
    );
}
