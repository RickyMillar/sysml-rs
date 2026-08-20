//! Integration tests for `sysml info` subcommand.

mod project_common;
use project_common::*;

use std::fs;

#[test]
fn info_shows_project_details() {
    let fixture = fixture_path("example-workspace/coffee-machine");

    let output = sysml_bin()
        .arg("info")
        .current_dir(&fixture)
        .output()
        .expect("failed to run sysml info");

    assert!(output.status.success(), "info should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Project: coffee-machine"));
    assert!(stdout.contains("Version: 0.1.0"));
    assert!(stdout.contains("License: MIT"));
    assert!(stdout.contains("IRI: urn:acme:coffee-machine"));
    assert!(stdout.contains("beverage-types"));
}

#[test]
fn info_json_output() {
    let fixture = fixture_path("example-workspace/coffee-machine");

    let output = sysml_bin()
        .args(["info", "--json"])
        .current_dir(&fixture)
        .output()
        .expect("failed to run sysml info --json");

    assert!(output.status.success(), "info --json should succeed");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("should be valid JSON");
    assert_eq!(json["name"], "coffee-machine");
    assert_eq!(json["version"], "0.1.0");
    assert_eq!(json["iri"], "urn:acme:coffee-machine");
    assert!(json["dependencies"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("beverage-types")));
    assert!(json["stdlib"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("analysis")));
}

#[test]
fn info_fails_without_manifest() {
    let dir = temp_dir("info-no-manifest");

    let output = sysml_bin()
        .arg("info")
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml info");

    assert!(
        !output.status.success(),
        "info should fail without sysml.toml"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no sysml.toml found"),
        "should mention missing manifest"
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn info_discovers_manifest_from_subdirectory() {
    let dir = temp_dir("info-subdir");
    let sub = dir.join("deep/nested/dir");
    fs::create_dir_all(&sub).unwrap();
    fs::write(
        dir.join("sysml.toml"),
        "[project]\nname = \"parent-project\"\nversion = \"2.0.0\"\n",
    )
    .unwrap();

    let output = sysml_bin()
        .arg("info")
        .current_dir(&sub)
        .output()
        .expect("failed to run sysml info");

    assert!(output.status.success(), "should discover parent manifest");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Project: parent-project"));
    assert!(stdout.contains("Version: 2.0.0"));

    let _ = fs::remove_dir_all(dir);
}
