//! Integration tests for `sysml package` subcommand.

mod project_common;
use project_common::*;

use std::fs;

#[test]
fn package_creates_kpar_archive() {
    let dir = temp_dir("package-basic");
    let src_dir = dir.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(
        dir.join("sysml.toml"),
        "[project]\nname = \"packaged-model\"\nversion = \"1.2.3\"\ndescription = \"A test package\"\n",
    )
    .unwrap();
    fs::write(
        src_dir.join("model.sysml"),
        "package PackagedModel {\n    part def Widget;\n}\n",
    )
    .unwrap();

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
    assert!(stdout.contains("Packaged packaged-model"));
    assert!(stdout.contains("1 files"));

    // KPAR file should exist
    let kpar_path = dir.join("target/package/packaged-model-1.2.3.kpar");
    assert!(
        kpar_path.exists(),
        "KPAR file should be created at {}",
        kpar_path.display()
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn package_custom_output_dir() {
    let dir = temp_dir("package-output");
    let src_dir = dir.join("src");
    let out_dir = dir.join("dist");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(
        dir.join("sysml.toml"),
        "[project]\nname = \"custom-out\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(src_dir.join("main.sysml"), "package CustomOut {}\n").unwrap();

    let output = sysml_bin()
        .args(["package", "--output", out_dir.to_str().unwrap()])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml package");

    assert!(output.status.success(), "package --output should succeed");

    let kpar_path = out_dir.join("custom-out-0.1.0.kpar");
    assert!(kpar_path.exists(), "KPAR should be at custom output path");

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn package_kpar_contains_valid_json() {
    let dir = temp_dir("package-json");
    let src_dir = dir.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(
        dir.join("sysml.toml"),
        "[project]\nname = \"json-test\"\nversion = \"2.0.0\"\ndescription = \"Testing JSON\"\n",
    )
    .unwrap();
    fs::write(
        src_dir.join("main.sysml"),
        "package JsonTest {\n    part def Sensor;\n}\n",
    )
    .unwrap();

    sysml_bin()
        .arg("package")
        .current_dir(&dir)
        .output()
        .unwrap();

    let kpar_path = dir.join("target/package/json-test-2.0.0.kpar");
    assert!(kpar_path.exists());

    // Read the KPAR and verify its contents
    let archive = sysml_project::kpar::read_kpar(&kpar_path).expect("should read generated KPAR");
    assert_eq!(archive.project_info.name, "json-test");
    assert_eq!(archive.project_info.version, "2.0.0");
    assert_eq!(
        archive.project_info.description.as_deref(),
        Some("Testing JSON")
    );

    // Default stdlib selection includes all standard libraries.
    assert!(
        archive.project_info.usage.len() >= 10,
        "should have all stdlib usage entries by default"
    );
    let resources: Vec<&str> = archive
        .project_info
        .usage
        .iter()
        .map(|u| u.resource.as_str())
        .collect();
    assert!(resources.iter().any(|r| r.contains("Semantic-Library")));
    assert!(resources
        .iter()
        .any(|r| r.contains("Analysis-Domain-Library")));

    // Metadata should have index
    assert!(
        archive.metadata.index.contains_key("JsonTest"),
        "meta index should contain package name"
    );
    assert_eq!(archive.metadata.index["JsonTest"], "main.sysml");

    // Source files
    assert_eq!(archive.source_files.len(), 1);
    assert_eq!(archive.source_files[0].0, "main.sysml");

    let _ = fs::remove_dir_all(dir);
}
