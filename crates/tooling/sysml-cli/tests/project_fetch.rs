//! Integration tests for `sysml fetch` + registry dependency tests.

mod project_common;
use project_common::*;

use std::fs;

#[test]
fn fetch_resolves_dependencies_without_writing_lock() {
    let dir = temp_dir("fetch-resolve");
    let dep = dir.join("lib-fetch");
    fs::create_dir_all(dep.join("src")).unwrap();
    fs::write(
        dep.join("sysml.toml"),
        "[project]\nname = \"lib-fetch\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(dep.join("src/lib.sysml"), "package LibFetch {}\n").unwrap();
    fs::write(
        dir.join("sysml.toml"),
        "[project]\nname = \"root\"\nversion = \"0.1.0\"\n\n[dependencies]\nlib-fetch = { path = \"./lib-fetch\" }\n",
    )
    .unwrap();

    let output = sysml_bin()
        .arg("fetch")
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml fetch");
    assert!(
        output.status.success(),
        "fetch should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Fetched 1 packages"));
    assert!(
        !dir.join("sysml.lock").exists(),
        "fetch should not write lock"
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn fetch_supports_json_and_quiet_output_modes() {
    let dir = temp_dir("fetch-json-quiet");
    let dep = dir.join("lib-fetch");
    fs::create_dir_all(dep.join("src")).unwrap();
    fs::write(
        dep.join("sysml.toml"),
        "[project]\nname = \"lib-fetch\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(dep.join("src/lib.sysml"), "package LibFetch {}\n").unwrap();
    fs::write(
        dir.join("sysml.toml"),
        "[project]\nname = \"root\"\nversion = \"0.1.0\"\n\n[dependencies]\nlib-fetch = { path = \"./lib-fetch\" }\n",
    )
    .unwrap();

    let json_out = sysml_bin()
        .args(["fetch", "--json"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml fetch --json");
    assert!(json_out.status.success());
    let parsed: serde_json::Value = serde_json::from_slice(&json_out.stdout).unwrap();
    assert_eq!(parsed["status"], "fetched");
    let packages = parsed["packages"]
        .as_array()
        .expect("packages must be array");
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0]["name"], "lib-fetch");
    assert_eq!(packages[0]["version"], "0.1.0");
    assert_eq!(packages[0]["source"], "path:./lib-fetch");
    assert!(
        !dir.join("sysml.lock").exists(),
        "fetch should not write lock in json mode"
    );

    let quiet_out = sysml_bin()
        .args(["fetch", "--quiet"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml fetch --quiet");
    assert!(quiet_out.status.success());
    assert!(
        String::from_utf8_lossy(&quiet_out.stdout).trim().is_empty(),
        "quiet mode should suppress stdout"
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn fetch_supports_registry_dependency_from_local_sysand_index() {
    let dir = temp_dir("fetch-registry-sysand");
    let index_dir = dir.join(".sysml/registries/sysand");
    fs::create_dir_all(index_dir.join("artifacts")).unwrap();

    let package = "reg-cli-lib";
    let version = "6.7.8";
    let artifact = index_dir.join(format!("artifacts/{package}-{version}.kpar"));
    write_fixture_kpar(&artifact, package, version);
    let checksum = format!("sha256:{}", sha256_hex_file(&artifact));
    clean_registry_cache_for_request("sysand", package, version);
    fs::write(
        index_dir.join("index.json"),
        format!(
            "{{\"packages\":{{\"{package}\":{{\"{version}\":{{\"artifact\":\"artifacts/{package}-{version}.kpar\",\"checksum\":\"{checksum}\"}}}}}}}}"
        ),
    )
    .unwrap();

    fs::write(
        dir.join("sysml.toml"),
        format!(
            "[project]\nname = \"root\"\nversion = \"0.1.0\"\n\n[dependencies]\n{package} = {{ version = \"{version}\", registry = \"sysand\" }}\n"
        ),
    )
    .unwrap();

    let output = sysml_bin()
        .args(["fetch", "--json"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml fetch --json for registry dependency");
    assert!(
        output.status.success(),
        "fetch should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["status"], "fetched");
    let packages = parsed["packages"]
        .as_array()
        .expect("packages must be array");
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0]["name"], package);
    assert_eq!(packages[0]["version"], version);
    assert_eq!(
        packages[0]["source"],
        format!("registry:sysand:{package}@{version}")
    );
    assert_eq!(packages[0]["requested_requirement"], version);
    assert_eq!(packages[0]["resolved_version"], version);
    assert!(
        !dir.join("sysml.lock").exists(),
        "fetch should not write lock for registry dependency"
    );

    clean_registry_cache_for_request("sysand", package, version);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn registry_range_json_outputs_include_requested_and_resolved_versions() {
    let dir = temp_dir("registry-range-json");
    let package = "reg-cli-range-lib";
    let requirement = "^1.4";
    let selected = "1.9.1";

    let artifact_142 = dir
        .join(".sysml/registries/sysand/artifacts")
        .join(format!("{package}-1.4.2.kpar"));
    let artifact_191 = dir
        .join(".sysml/registries/sysand/artifacts")
        .join(format!("{package}-1.9.1.kpar"));
    let artifact_200 = dir
        .join(".sysml/registries/sysand/artifacts")
        .join(format!("{package}-2.0.0.kpar"));
    fs::create_dir_all(artifact_142.parent().unwrap()).unwrap();
    write_fixture_kpar(&artifact_142, package, "1.4.2");
    write_fixture_kpar(&artifact_191, package, "1.9.1");
    write_fixture_kpar(&artifact_200, package, "2.0.0");
    write_sysand_index_with_releases(
        &dir,
        package,
        &[
            ("1.4.2", artifact_142.clone()),
            ("1.9.1", artifact_191.clone()),
            ("2.0.0", artifact_200.clone()),
        ],
    );
    clean_registry_cache_for_request("sysand", package, requirement);

    fs::write(
        dir.join("sysml.toml"),
        format!(
            "[project]\nname = \"root\"\nversion = \"0.1.0\"\n\n[dependencies]\n{package} = \"{requirement}\"\n"
        ),
    )
    .unwrap();

    let fetch_json = sysml_bin()
        .args(["fetch", "--json"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml fetch --json for registry range");
    assert!(
        fetch_json.status.success(),
        "fetch should succeed: {}",
        String::from_utf8_lossy(&fetch_json.stderr)
    );
    let parsed_fetch: serde_json::Value = serde_json::from_slice(&fetch_json.stdout).unwrap();
    let fetch_packages = parsed_fetch["packages"].as_array().unwrap();
    assert_eq!(fetch_packages.len(), 1);
    assert_eq!(fetch_packages[0]["name"], package);
    assert_eq!(
        fetch_packages[0]["source"],
        format!("registry:sysand:{package}@{selected}")
    );
    assert_eq!(fetch_packages[0]["requested_requirement"], requirement);
    assert_eq!(fetch_packages[0]["resolved_version"], selected);

    let lock_json = sysml_bin()
        .args(["lock", "--json"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml lock --json for registry range");
    assert!(lock_json.status.success());
    let parsed_lock: serde_json::Value = serde_json::from_slice(&lock_json.stdout).unwrap();
    assert_eq!(parsed_lock["status"], "updated");
    let lock_packages = parsed_lock["packages"].as_array().unwrap();
    assert_eq!(lock_packages.len(), 1);
    assert_eq!(lock_packages[0]["name"], package);
    assert_eq!(lock_packages[0]["requested_requirement"], requirement);
    assert_eq!(lock_packages[0]["resolved_version"], selected);
    assert_eq!(
        lock_packages[0]["source"],
        format!("registry:sysand:{package}@{selected}")
    );

    let lock_path = dir.join("sysml.lock");
    let first_lock = fs::read(&lock_path).unwrap();
    let lock_json_second = sysml_bin()
        .args(["lock", "--json"])
        .current_dir(&dir)
        .output()
        .expect("failed to run second sysml lock --json for registry range");
    assert!(lock_json_second.status.success());
    let parsed_second: serde_json::Value =
        serde_json::from_slice(&lock_json_second.stdout).unwrap();
    assert_eq!(parsed_second["status"], "up_to_date");
    let second_lock = fs::read(&lock_path).unwrap();
    assert_eq!(
        first_lock, second_lock,
        "lock bytes should remain deterministic"
    );

    let update_json = sysml_bin()
        .args(["update", "--json"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml update --json for registry range");
    assert!(update_json.status.success());
    let parsed_update: serde_json::Value = serde_json::from_slice(&update_json.stdout).unwrap();
    let update_packages = parsed_update["packages"].as_array().unwrap();
    assert_eq!(update_packages[0]["requested_requirement"], requirement);
    assert_eq!(update_packages[0]["resolved_version"], selected);

    let tree_json = sysml_bin()
        .args(["tree", "--json"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml tree --json for registry range");
    assert!(tree_json.status.success());
    let parsed_tree: serde_json::Value = serde_json::from_slice(&tree_json.stdout).unwrap();
    let tree_packages = parsed_tree["packages"].as_array().unwrap();
    let registry_pkg = tree_packages
        .iter()
        .find(|pkg| pkg["name"] == package)
        .expect("tree should include registry package metadata");
    assert_eq!(registry_pkg["requested_requirement"], requirement);
    assert_eq!(registry_pkg["resolved_version"], selected);

    clean_registry_cache_for_request("sysand", package, requirement);
    let _ = fs::remove_dir_all(dir);
}
