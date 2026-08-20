//! End-to-end workflow tests, help text tests, and fixture-based tests.

mod project_common;
use project_common::*;

use std::fs;

// ── End-to-end workflow ─────────────────────────────────────

#[test]
fn full_workflow_init_add_lock_info_package() {
    let root = temp_dir("e2e-workflow");

    // 1. Create a library project
    let lib_dir = root.join("my-lib");
    fs::create_dir_all(lib_dir.join("src")).unwrap();
    fs::write(
        lib_dir.join("sysml.toml"),
        "[project]\nname = \"my-lib\"\nversion = \"0.5.0\"\n",
    )
    .unwrap();
    fs::write(
        lib_dir.join("src/lib.sysml"),
        "package MyLib {\n    item def Gadget;\n}\n",
    )
    .unwrap();

    // 2. Init main project
    let output = sysml_bin()
        .args(["init", "--name", "my-app"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(output.status.success(), "init should succeed");

    let app_dir = root.join("my-app");
    assert!(app_dir.join("sysml.toml").exists());

    // 3. Add dependency
    let output = sysml_bin()
        .args(["add", "my-lib", "--path", "../my-lib"])
        .current_dir(&app_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "add should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // 4. Lock should show the dep
    let output = sysml_bin()
        .arg("lock")
        .current_dir(&app_dir)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("up to date") || stdout.contains("Resolved"),
        "lock should work"
    );

    // 5. Info should show the dep
    let output = sysml_bin()
        .arg("info")
        .current_dir(&app_dir)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("my-lib"), "info should show dep");

    // 6. Package should produce a .kpar
    let output = sysml_bin()
        .arg("package")
        .current_dir(&app_dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "package should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Packaged my-app"));

    let kpar = app_dir.join("target/package/my-app-0.1.0.kpar");
    assert!(kpar.exists(), "KPAR should exist");

    // 7. Remove dep and verify it's gone
    let output = sysml_bin()
        .args(["remove", "my-lib"])
        .current_dir(&app_dir)
        .output()
        .unwrap();
    assert!(output.status.success());

    let manifest = fs::read_to_string(app_dir.join("sysml.toml")).unwrap();
    assert!(!manifest.contains("my-lib"), "dep should be removed");

    let _ = fs::remove_dir_all(root);
}

// ── Help text ───────────────────────────────────────────────

#[test]
fn help_lists_project_commands() {
    let output = sysml_bin()
        .arg("--help")
        .output()
        .expect("failed to run sysml --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("init"), "help should list init");
    assert!(stdout.contains("info"), "help should list info");
    assert!(stdout.contains("add"), "help should list add");
    assert!(stdout.contains("remove"), "help should list remove");
    assert!(stdout.contains("lock"), "help should list lock");
    assert!(stdout.contains("fetch"), "help should list fetch");
    assert!(stdout.contains("update"), "help should list update");
    assert!(stdout.contains("tree"), "help should list tree");
    assert!(stdout.contains("why"), "help should list why");
    assert!(stdout.contains("cache"), "help should list cache");
    assert!(stdout.contains("package"), "help should list package");
}

#[test]
fn init_help_works() {
    let output = sysml_bin()
        .args(["init", "--help"])
        .output()
        .expect("failed to run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--name"));
}

#[test]
fn package_help_works() {
    let output = sysml_bin()
        .args(["package", "--help"])
        .output()
        .expect("failed to run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--output"));
    assert!(stdout.contains(".kpar"));
}

// ── Fixture-based tests ─────────────────────────────────────

#[test]
fn info_on_example_workspace() {
    let fixture = fixture_path("example-workspace");

    let output = sysml_bin()
        .arg("info")
        .current_dir(&fixture)
        .output()
        .expect("failed to run sysml info");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("beverage-workspace"));
    assert!(stdout.contains("Workspace Members:"));
    assert!(stdout.contains("beverage-types"));
    assert!(stdout.contains("coffee-machine"));
}

#[test]
fn package_example_coffee_machine() {
    let fixture = fixture_path("example-workspace/coffee-machine");

    let dir = temp_dir("package-fixture");
    // Copy fixture to temp so we can write target/ there
    copy_dir_recursive(&fixture, &dir);

    // Also need beverage-types next to it for the path dep
    let bev_src = fixture_path("example-workspace/beverage-types");
    let bev_dst = dir.parent().unwrap().join("beverage-types");
    if !bev_dst.exists() {
        copy_dir_recursive(&bev_src, &bev_dst);
    }

    let output = sysml_bin()
        .arg("package")
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml package");

    assert!(
        output.status.success(),
        "package should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Packaged coffee-machine"));

    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(bev_dst);
}
