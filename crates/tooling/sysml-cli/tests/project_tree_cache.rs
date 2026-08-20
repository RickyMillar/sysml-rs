//! Integration tests for `sysml tree` / `sysml why` / `sysml cache` subcommands.

mod project_common;
use project_common::*;

use std::fs;

#[test]
fn tree_and_why_report_dependency_path() {
    let dir = temp_dir("tree-why");
    let mid = dir.join("mid-lib");
    let leaf = dir.join("leaf-lib");
    fs::create_dir_all(mid.join("src")).unwrap();
    fs::create_dir_all(leaf.join("src")).unwrap();

    fs::write(
        leaf.join("sysml.toml"),
        "[project]\nname = \"leaf-lib\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(leaf.join("src/leaf.sysml"), "package Leaf {}\n").unwrap();

    fs::write(
        mid.join("sysml.toml"),
        "[project]\nname = \"mid-lib\"\nversion = \"0.1.0\"\n\n[dependencies]\nleaf-lib = { path = \"../leaf-lib\" }\n",
    )
    .unwrap();
    fs::write(mid.join("src/mid.sysml"), "package Mid {}\n").unwrap();

    fs::write(
        dir.join("sysml.toml"),
        "[project]\nname = \"root\"\nversion = \"0.1.0\"\n\n[dependencies]\nmid-lib = { path = \"./mid-lib\" }\n",
    )
    .unwrap();

    let tree_out = sysml_bin()
        .arg("tree")
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml tree");
    assert!(tree_out.status.success());
    let tree_stdout = String::from_utf8_lossy(&tree_out.stdout);
    assert!(tree_stdout.contains("root"));
    assert!(tree_stdout.contains("mid-lib"));
    assert!(tree_stdout.contains("leaf-lib"));

    let why_out = sysml_bin()
        .args(["why", "leaf-lib"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml why");
    assert!(why_out.status.success());
    let why_stdout = String::from_utf8_lossy(&why_out.stdout);
    assert!(why_stdout.contains("root -> mid-lib -> leaf-lib"));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn tree_and_why_support_json_and_quiet_output_modes() {
    let dir = temp_dir("tree-why-json-quiet");
    let mid = dir.join("mid-lib");
    let leaf = dir.join("leaf-lib");
    fs::create_dir_all(mid.join("src")).unwrap();
    fs::create_dir_all(leaf.join("src")).unwrap();

    fs::write(
        leaf.join("sysml.toml"),
        "[project]\nname = \"leaf-lib\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(leaf.join("src/leaf.sysml"), "package Leaf {}\n").unwrap();

    fs::write(
        mid.join("sysml.toml"),
        "[project]\nname = \"mid-lib\"\nversion = \"0.1.0\"\n\n[dependencies]\nleaf-lib = { path = \"../leaf-lib\" }\n",
    )
    .unwrap();
    fs::write(mid.join("src/mid.sysml"), "package Mid {}\n").unwrap();

    fs::write(
        dir.join("sysml.toml"),
        "[project]\nname = \"root\"\nversion = \"0.1.0\"\n\n[dependencies]\nmid-lib = { path = \"./mid-lib\" }\n",
    )
    .unwrap();

    let tree_json = sysml_bin()
        .args(["tree", "--json"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml tree --json");
    assert!(tree_json.status.success());
    let parsed_tree: serde_json::Value = serde_json::from_slice(&tree_json.stdout).unwrap();
    assert_eq!(parsed_tree["root"], "root");
    let edges = parsed_tree["edges"]
        .as_object()
        .expect("tree edges must be an object");
    assert_eq!(edges["root"], serde_json::json!(["mid-lib"]));
    assert_eq!(edges["mid-lib"], serde_json::json!(["leaf-lib"]));
    let packages = parsed_tree["packages"]
        .as_array()
        .expect("tree packages must be an array");
    assert!(
        packages.iter().any(|pkg| pkg["name"] == "mid-lib"),
        "tree JSON must include mid-lib package metadata"
    );
    assert!(
        packages.iter().any(|pkg| pkg["name"] == "leaf-lib"),
        "tree JSON must include leaf-lib package metadata"
    );

    let tree_quiet = sysml_bin()
        .args(["tree", "--quiet"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml tree --quiet");
    assert!(tree_quiet.status.success());
    assert!(
        String::from_utf8_lossy(&tree_quiet.stdout)
            .trim()
            .is_empty(),
        "tree --quiet should suppress stdout"
    );

    let why_json = sysml_bin()
        .args(["why", "leaf-lib", "--json"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml why --json");
    assert!(why_json.status.success());
    let parsed_why: serde_json::Value = serde_json::from_slice(&why_json.stdout).unwrap();
    assert_eq!(parsed_why["target"], "leaf-lib");
    assert_eq!(
        parsed_why["path"],
        serde_json::json!(["root", "mid-lib", "leaf-lib"])
    );

    let why_quiet = sysml_bin()
        .args(["why", "leaf-lib", "--quiet"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml why --quiet");
    assert!(why_quiet.status.success());
    assert!(
        String::from_utf8_lossy(&why_quiet.stdout).trim().is_empty(),
        "why --quiet should suppress stdout"
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn cache_clean_removes_dependency_cache_dir() {
    if !git_available() {
        eprintln!("skipping cache clean test: git binary unavailable");
        return;
    }

    let remote_root = temp_dir("cache-clean-remote");
    let remote_repo = remote_root.join("remote-lib");
    let commit = init_git_repo_with_manifest(&remote_repo, "0.4.0");
    let url = format!("file://{}", remote_repo.canonicalize().unwrap().display());

    let project_dir = temp_dir("cache-clean-project");
    let cache_dir = temp_dir("cache-clean-cache");
    fs::write(
        project_dir.join("sysml.toml"),
        format!(
            "[project]\nname = \"root\"\nversion = \"1.0.0\"\n\n[dependencies]\nremote-lib = {{ git = \"{url}\", rev = \"{commit}\" }}\n"
        ),
    )
    .unwrap();

    let first = sysml_bin()
        .arg("lock")
        .env("SYSML_RS_CACHE_DIR", &cache_dir)
        .current_dir(&project_dir)
        .output()
        .expect("failed to populate dependency cache");
    assert!(first.status.success());
    assert!(
        cache_dir.join("dependencies").exists(),
        "expected cache dependencies dir to be populated"
    );

    let clean = sysml_bin()
        .args(["cache", "clean"])
        .env("SYSML_RS_CACHE_DIR", &cache_dir)
        .current_dir(&project_dir)
        .output()
        .expect("failed to run cache clean");
    assert!(clean.status.success());
    assert!(
        !cache_dir.join("dependencies").exists(),
        "cache clean should remove dependencies cache"
    );

    let _ = fs::remove_dir_all(project_dir);
    let _ = fs::remove_dir_all(remote_root);
    let _ = fs::remove_dir_all(cache_dir);
}

#[test]
fn cache_clean_supports_json_and_quiet_output_modes() {
    let cwd = temp_dir("cache-clean-json-quiet-cwd");
    let cache_dir = temp_dir("cache-clean-json-quiet-cache");
    let deps_root = cache_dir.join("dependencies/git/test");
    fs::create_dir_all(&deps_root).unwrap();
    fs::write(deps_root.join("dummy.txt"), "x").unwrap();

    let clean_json = sysml_bin()
        .args(["cache", "clean", "--json"])
        .env("SYSML_RS_CACHE_DIR", &cache_dir)
        .current_dir(&cwd)
        .output()
        .expect("failed to run sysml cache clean --json");
    assert!(clean_json.status.success());
    let parsed: serde_json::Value = serde_json::from_slice(&clean_json.stdout).unwrap();
    assert_eq!(parsed["removed"], true);
    assert_eq!(parsed["scope"], "dependencies");
    assert!(
        parsed["path"]
            .as_str()
            .map(|v| v.ends_with("/dependencies"))
            .unwrap_or(false),
        "cache clean JSON path should target dependencies scope"
    );
    assert!(
        !cache_dir.join("dependencies").exists(),
        "dependencies cache should be removed after clean"
    );

    let clean_quiet = sysml_bin()
        .args(["cache", "clean", "--quiet"])
        .env("SYSML_RS_CACHE_DIR", &cache_dir)
        .current_dir(&cwd)
        .output()
        .expect("failed to run sysml cache clean --quiet");
    assert!(clean_quiet.status.success());
    assert!(
        String::from_utf8_lossy(&clean_quiet.stdout)
            .trim()
            .is_empty(),
        "cache clean --quiet should suppress stdout"
    );

    let _ = fs::remove_dir_all(cwd);
    let _ = fs::remove_dir_all(cache_dir);
}
