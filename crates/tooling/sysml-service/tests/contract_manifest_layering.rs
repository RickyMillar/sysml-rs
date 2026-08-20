//! Manifest layering proof (P6).
//!
//! The file-loading model promises that adding a zero-dep `sysml.toml`
//! to a folder is a **purely additive** change — every behaviour you
//! got from the bare folder still holds. These tests prove that:
//!
//! 1. The set of loaded URIs is identical with and without a manifest.
//! 2. Per-file diagnostics for the same source are identical.
//! 3. Nested manifests are **isolated** from the outer project
//!    (Cargo-style boundary), confirming the existing
//!    `open_folder_isolates_nested_subprojects` unit test at the
//!    contract-test level.
//!
//! The richer scenario E from the model doc (outer + two nested
//! manifests resolving simultaneously, each with its own project id)
//! is gated on a workspace-level entry point that ingests multiple
//! manifest roots at once. Tracked separately.

use std::collections::HashSet;
use std::fs;
use sysml_project::discovery::OpenTarget;
use sysml_service::SysmlService;
use tempfile::TempDir;

const MIN_MANIFEST: &str = r#"
[project]
name = "p6-test"
version = "0.1.0"
"#;

fn scratch(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().unwrap();
    for (rel, content) in files {
        let p = dir.path().join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&p, content).unwrap();
    }
    dir
}

fn uri_basename(uri: &str) -> &str {
    uri.rsplit(['/', '\\']).next().unwrap_or(uri)
}

#[test]
fn manifest_is_additive_loaded_uris() {
    // Two parallel scenarios — exact same source files, only difference
    // is the presence of a zero-dep sysml.toml.
    let files = &[
        ("a.sysml", "package A { part def Widget; }"),
        ("b.sysml", "package B { import A::*; part w : Widget; }"),
    ];
    let bare = scratch(files);
    let manifested = {
        let dir = scratch(files);
        fs::write(dir.path().join("sysml.toml"), MIN_MANIFEST).unwrap();
        dir
    };

    let svc_bare = SysmlService::empty();
    let ctx_bare = svc_bare
        .open_context(OpenTarget::Folder(bare.path().to_path_buf()))
        .expect("open bare");

    let svc_mani = SysmlService::empty();
    let ctx_mani = svc_mani
        .open_context(OpenTarget::Folder(manifested.path().to_path_buf()))
        .expect("open manifested");

    let bare_names: HashSet<&str> = ctx_bare.loaded_uris.iter().map(|u| uri_basename(u)).collect();
    let mani_names: HashSet<&str> = ctx_mani.loaded_uris.iter().map(|u| uri_basename(u)).collect();

    assert_eq!(
        bare_names, mani_names,
        "manifest must not change which files load; bare={bare_names:?} manifested={mani_names:?}"
    );
}

#[test]
fn manifest_is_additive_diagnostics() {
    // Same two-file scenario; this time we check the per-file
    // diagnostic set on `b.sysml` (which imports from `a.sysml`).
    // With or without manifest, `b.sysml` should see `Widget` and
    // resolve cleanly — no IM010, no E200.
    let files = &[
        ("a.sysml", "package A { part def Widget; }"),
        ("b.sysml", "package B { import A::*; part w : Widget; }"),
    ];

    let bare = scratch(files);
    let manifested = {
        let dir = scratch(files);
        fs::write(dir.path().join("sysml.toml"), MIN_MANIFEST).unwrap();
        dir
    };

    let svc_bare = SysmlService::empty();
    svc_bare
        .open_context(OpenTarget::Folder(bare.path().to_path_buf()))
        .expect("open bare");
    let bare_b_uri = bare.path().join("b.sysml").to_string_lossy().into_owned();
    let bare_diags = svc_bare.diagnostics(&bare_b_uri).expect("bare diags");

    let svc_mani = SysmlService::empty();
    svc_mani
        .open_context(OpenTarget::Folder(manifested.path().to_path_buf()))
        .expect("open manifested");
    let mani_b_uri = manifested
        .path()
        .join("b.sysml")
        .to_string_lossy()
        .into_owned();
    let mani_diags = svc_mani.diagnostics(&mani_b_uri).expect("mani diags");

    let codes_bare: HashSet<Option<String>> = bare_diags.iter().map(|d| d.code.clone()).collect();
    let codes_mani: HashSet<Option<String>> = mani_diags.iter().map(|d| d.code.clone()).collect();

    assert_eq!(
        codes_bare, codes_mani,
        "manifest must not change the diagnostic code set; bare={codes_bare:?} manifested={codes_mani:?}"
    );

    // And — both should resolve cleanly: no unresolved-name signal.
    assert!(
        !codes_bare.contains(&Some("E200".to_string()))
            && !codes_bare.contains(&Some("IM010".to_string())),
        "Widget should resolve cross-file even without manifest: {codes_bare:?}"
    );
}

#[test]
fn nested_manifest_excluded_from_outer_project() {
    // Outer folder (bare) contains a file `outer.sysml` and a sub-dir
    // `sub/` with its own sysml.toml + `inner.sysml`. Opening the
    // outer folder should ONLY load outer.sysml — Cargo-style.
    //
    // This is the contract-level companion to the unit test
    // `open_folder_isolates_nested_subprojects` in open_context.rs.
    let dir = scratch(&[
        ("outer.sysml", "package Outer;"),
        ("sub/sysml.toml", MIN_MANIFEST),
        ("sub/inner.sysml", "package Inner;"),
    ]);
    let svc = SysmlService::empty();
    let ctx = svc
        .open_context(OpenTarget::Folder(dir.path().to_path_buf()))
        .expect("open outer");

    assert_eq!(
        ctx.loaded_uris.len(),
        1,
        "outer project must not pull files from nested manifest; got {:?}",
        ctx.loaded_uris
    );
    assert!(
        ctx.loaded_uris[0].ends_with("outer.sysml"),
        "wrong file loaded: {}",
        ctx.loaded_uris[0]
    );
}
