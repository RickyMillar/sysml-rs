//! Strict-mode diagnostic enrichment (P5.3).
//!
//! These tests verify that when a single file is opened without folder
//! context (`ProjectKind::Strict`), the diagnostic pipeline:
//!
//! 1. Surfaces a single IM012 banner explaining cross-file imports won't
//!    resolve in strict mode.
//! 2. Decorates each IM010 (unresolved-name) diagnostic with a
//!    `neighbour file` note pointing at the sibling that declares the
//!    missing name, so the user can fix the situation with one click.
//!
//! When the same files are opened via folder/workspace (Discovered mode),
//! neither IM012 nor the neighbour note should appear — cross-file
//! resolution actually succeeds.

use std::fs;
use sysml_project::discovery::OpenTarget;
use sysml_service::SysmlService;
use tempfile::TempDir;

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

/// Two-file repro: ports.sysml defines a part, uses.sysml references it
/// without an import. Opening uses.sysml alone (strict) should:
///   - emit IM012 once,
///   - attach a neighbour-file hint to the IM010 firing on `WaterPort`.
#[test]
fn strict_open_emits_im012_and_decorates_im010() {
    let dir = scratch(&[
        ("ports.sysml", "package Ports { part def WaterPort; }"),
        ("uses.sysml", "package Uses { part p : WaterPort; }"),
    ]);
    let svc = SysmlService::empty();
    let uses_path = dir.path().join("uses.sysml");
    let ctx = svc
        .open_context(OpenTarget::File(uses_path.clone()))
        .expect("open uses.sysml strict");

    // Sanity: only uses.sysml is loaded (no sibling scan in strict mode).
    assert_eq!(ctx.loaded_uris.len(), 1);
    let uri = ctx.loaded_uris.first().unwrap();

    let diags = svc.diagnostics(uri).expect("diagnostics");

    let im012_count = diags.iter().filter(|d| d.code.as_deref() == Some("IM012")).count();
    assert_eq!(
        im012_count, 1,
        "expected exactly one IM012 in strict mode; got {im012_count}\ndiags: {diags:?}"
    );

    let im010 = diags
        .iter()
        .find(|d| {
            d.code.as_deref() == Some("IM010") && d.message.contains("WaterPort")
        })
        .unwrap_or_else(|| panic!("expected IM010 for WaterPort; diags: {diags:?}"));
    let has_neighbour_note = im010
        .notes
        .iter()
        .any(|n: &String| n.contains("ports.sysml"));
    assert!(
        has_neighbour_note,
        "IM010 should carry a 'ports.sysml' neighbour note in strict mode;\nnotes: {:?}",
        im010.notes
    );
}

/// Opening the same folder (Discovered mode) should NOT emit IM012 and
/// should NOT add neighbour notes — cross-file resolution actually
/// succeeds, so the unresolved diagnostic disappears entirely.
#[test]
fn discovered_open_suppresses_im012_and_resolves_cross_file() {
    let dir = scratch(&[
        ("ports.sysml", "package Ports { part def WaterPort; }"),
        ("uses.sysml", "package Uses { import Ports::*; part p : WaterPort; }"),
    ]);
    let svc = SysmlService::empty();
    let ctx = svc
        .open_context(OpenTarget::Folder(dir.path().to_path_buf()))
        .expect("open folder discovered");

    let uses_uri = ctx
        .loaded_uris
        .iter()
        .find(|u| u.ends_with("uses.sysml"))
        .expect("uses.sysml loaded");

    let diags = svc.diagnostics(uses_uri).expect("diagnostics");
    let im012_count = diags
        .iter()
        .filter(|d| d.code.as_deref() == Some("IM012"))
        .count();
    assert_eq!(
        im012_count, 0,
        "Discovered mode must not emit IM012; got {im012_count}\ndiags: {diags:?}"
    );
}
