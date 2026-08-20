//! Integration tests for `sysml init` subcommand.

mod project_common;
use project_common::*;

use std::fs;

#[test]
fn init_creates_project_in_current_dir() {
    let dir = temp_dir("init-cwd");

    let output = sysml_bin()
        .arg("init")
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml init");

    assert!(output.status.success(), "init should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Created SysML project"),
        "should print creation message"
    );

    // sysml.toml should exist
    let manifest = dir.join("sysml.toml");
    assert!(manifest.exists(), "sysml.toml should be created");

    // src/main.sysml should exist
    let main_sysml = dir.join("src/main.sysml");
    assert!(main_sysml.exists(), "src/main.sysml should be created");

    // sysml.toml should contain the dir name as project name
    let content = fs::read_to_string(&manifest).unwrap();
    assert!(content.contains("[project]"));
    assert!(content.contains("version = \"0.1.0\""));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn init_with_name_creates_new_directory() {
    let parent = temp_dir("init-name");

    let output = sysml_bin()
        .args(["init", "--name", "my-cool-project"])
        .current_dir(&parent)
        .output()
        .expect("failed to run sysml init");

    assert!(output.status.success(), "init --name should succeed");

    let project_dir = parent.join("my-cool-project");
    assert!(project_dir.join("sysml.toml").exists());
    assert!(project_dir.join("src/main.sysml").exists());

    let content = fs::read_to_string(project_dir.join("sysml.toml")).unwrap();
    assert!(content.contains("name = \"my-cool-project\""));

    // The generated .sysml file should use PascalCase package name
    let sysml_content = fs::read_to_string(project_dir.join("src/main.sysml")).unwrap();
    assert!(sysml_content.contains("package MyCoolProject"));

    let _ = fs::remove_dir_all(parent);
}

#[test]
fn init_fails_if_manifest_already_exists() {
    let dir = temp_dir("init-exists");
    fs::write(
        dir.join("sysml.toml"),
        "[project]\nname = \"existing\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();

    let output = sysml_bin()
        .arg("init")
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml init");

    assert!(
        !output.status.success(),
        "init should fail if sysml.toml exists"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("already exists"),
        "should mention existing file"
    );

    let _ = fs::remove_dir_all(dir);
}
