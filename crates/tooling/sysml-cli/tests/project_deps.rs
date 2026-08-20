//! Integration tests for `sysml add` / `sysml remove` subcommands.

mod project_common;
use project_common::*;

use std::fs;

#[test]
fn add_path_dependency() {
    let dir = temp_dir("add-path");
    let dep_dir = dir.join("my-lib");
    fs::create_dir_all(dep_dir.join("src")).unwrap();
    fs::write(
        dep_dir.join("sysml.toml"),
        "[project]\nname = \"my-lib\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(dep_dir.join("src/lib.sysml"), "package MyLib {}\n").unwrap();

    // Init the main project
    sysml_bin()
        .arg("init")
        .current_dir(&dir)
        .output()
        .expect("init failed");

    // Add dependency
    let output = sysml_bin()
        .args(["add", "my-lib", "--path", "./my-lib"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml add");

    assert!(
        output.status.success(),
        "add should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Added 'my-lib'"));

    // sysml.toml should contain the dependency
    let manifest = fs::read_to_string(dir.join("sysml.toml")).unwrap();
    assert!(manifest.contains("[dependencies.my-lib]") || manifest.contains("my-lib"));
    assert!(manifest.contains("./my-lib"));

    // sysml.lock should be generated
    let lock_path = dir.join("sysml.lock");
    assert!(lock_path.exists(), "sysml.lock should be created");
    let lock = fs::read_to_string(&lock_path).unwrap();
    assert!(lock.contains("my-lib"));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn add_then_remove_dependency() {
    let dir = temp_dir("add-remove");
    let dep_dir = dir.join("dep-a");
    fs::create_dir_all(dep_dir.join("src")).unwrap();
    fs::write(
        dep_dir.join("sysml.toml"),
        "[project]\nname = \"dep-a\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(dep_dir.join("src/a.sysml"), "package DepA {}\n").unwrap();

    sysml_bin().arg("init").current_dir(&dir).output().unwrap();

    // Add
    sysml_bin()
        .args(["add", "dep-a", "--path", "./dep-a"])
        .current_dir(&dir)
        .output()
        .unwrap();

    let manifest_before = fs::read_to_string(dir.join("sysml.toml")).unwrap();
    assert!(
        manifest_before.contains("dep-a"),
        "dep-a should be in manifest"
    );

    // Remove
    let output = sysml_bin()
        .args(["remove", "dep-a"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml remove");

    assert!(output.status.success(), "remove should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Removed 'dep-a'"));

    let manifest_after = fs::read_to_string(dir.join("sysml.toml")).unwrap();
    assert!(
        !manifest_after.contains("dep-a"),
        "dep-a should be removed from manifest"
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn add_fails_when_dependency_resolution_fails() {
    let dir = temp_dir("add-fails-on-resolve");
    sysml_bin().arg("init").current_dir(&dir).output().unwrap();

    let output = sysml_bin()
        .args(["add", "missing-lib", "--path", "./missing-lib"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml add");

    assert!(
        !output.status.success(),
        "add should fail when lock resolution fails"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("dependency resolution failed"),
        "stderr should explain lock resolution failure"
    );

    let manifest = fs::read_to_string(dir.join("sysml.toml")).unwrap();
    assert!(
        !manifest.contains("missing-lib"),
        "failed add should not persist dependency change"
    );
    assert!(
        !dir.join("sysml.lock").exists(),
        "failed add should not create lock file"
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn remove_fails_when_updated_manifest_cannot_resolve() {
    let dir = temp_dir("remove-fails-on-resolve");
    let good_dep = dir.join("good-lib");
    fs::create_dir_all(good_dep.join("src")).unwrap();
    fs::write(
        good_dep.join("sysml.toml"),
        "[project]\nname = \"good-lib\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(good_dep.join("src/lib.sysml"), "package GoodLib {}\n").unwrap();

    fs::write(
        dir.join("sysml.toml"),
        r#"[project]
name = "main"
version = "1.0.0"

[dependencies]
good-lib = { path = "./good-lib" }
broken-lib = { path = "./missing-lib" }
"#,
    )
    .unwrap();

    let output = sysml_bin()
        .args(["remove", "good-lib"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml remove");

    assert!(
        !output.status.success(),
        "remove should fail when updated manifest cannot resolve"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("dependency resolution failed"),
        "stderr should explain lock resolution failure"
    );

    let manifest = fs::read_to_string(dir.join("sysml.toml")).unwrap();
    assert!(
        manifest.contains("good-lib") && manifest.contains("broken-lib"),
        "failed remove should not persist dependency deletion"
    );
    assert!(
        !dir.join("sysml.lock").exists(),
        "failed remove should not create lock file"
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn remove_nonexistent_dependency_fails() {
    let dir = temp_dir("remove-missing");
    fs::write(
        dir.join("sysml.toml"),
        "[project]\nname = \"test\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let output = sysml_bin()
        .args(["remove", "does-not-exist"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml remove");

    assert!(
        !output.status.success(),
        "remove should fail for nonexistent dep"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not found"), "should mention dep not found");

    let _ = fs::remove_dir_all(dir);
}
