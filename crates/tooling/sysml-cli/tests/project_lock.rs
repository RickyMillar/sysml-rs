//! Integration tests for `sysml lock` / `sysml update` subcommands.

mod project_common;
use project_common::*;

use std::fs;

#[test]
fn lock_resolves_dependencies() {
    let dir = temp_dir("lock-resolve");
    let dep = dir.join("lib-x");
    fs::create_dir_all(dep.join("src")).unwrap();
    fs::write(
        dep.join("sysml.toml"),
        "[project]\nname = \"lib-x\"\nversion = \"0.3.0\"\n",
    )
    .unwrap();
    fs::write(dep.join("src/x.sysml"), "package LibX {}\n").unwrap();

    fs::write(
        dir.join("sysml.toml"),
        "[project]\nname = \"main\"\nversion = \"1.0.0\"\n\n[dependencies]\nlib-x = { path = \"./lib-x\" }\n",
    )
    .unwrap();

    let output = sysml_bin()
        .arg("lock")
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml lock");

    assert!(
        output.status.success(),
        "lock should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Resolved 1 packages"));
    assert!(stdout.contains("lib-x"));

    let lock_path = dir.join("sysml.lock");
    assert!(lock_path.exists(), "sysml.lock should be created");
    let lock = fs::read_to_string(&lock_path).unwrap();
    assert!(lock.contains("lib-x"));
    assert!(lock.contains("0.3.0"));
    assert!(lock.contains("path:./lib-x"));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn lock_detects_up_to_date() {
    let dir = temp_dir("lock-uptodate");
    fs::write(
        dir.join("sysml.toml"),
        "[project]\nname = \"nodeps\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();

    // First lock
    sysml_bin().arg("lock").current_dir(&dir).output().unwrap();

    // Second lock should say up to date
    let output = sysml_bin()
        .arg("lock")
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml lock");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("up to date"),
        "should detect lock is current"
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn lock_resolves_git_rev_dependency() {
    if !git_available() {
        eprintln!("skipping git lock test: git binary unavailable");
        return;
    }

    let remote_root = temp_dir("lock-git-rev-remote");
    let remote_repo = remote_root.join("remote-lib");
    let commit = init_git_repo_with_manifest(&remote_repo, "0.4.0");
    let url = format!("file://{}", remote_repo.canonicalize().unwrap().display());

    let project_dir = temp_dir("lock-git-rev");
    let cache_dir = temp_dir("lock-git-rev-cache");
    fs::write(
        project_dir.join("sysml.toml"),
        format!(
            "[project]\nname = \"root\"\nversion = \"1.0.0\"\n\n[dependencies]\nremote-lib = {{ git = \"{url}\", rev = \"{commit}\" }}\n"
        ),
    )
    .unwrap();

    let output = sysml_bin()
        .arg("lock")
        .env("SYSML_RS_CACHE_DIR", &cache_dir)
        .current_dir(&project_dir)
        .output()
        .expect("failed to run sysml lock for git rev dependency");

    assert!(
        output.status.success(),
        "lock should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let lock_path = project_dir.join("sysml.lock");
    let lock = fs::read_to_string(&lock_path).unwrap();
    assert!(lock.contains("remote-lib"));
    assert!(lock.contains("0.4.0"));
    assert!(lock.contains(&format!("git:{url}#{commit}")));

    let _ = fs::remove_dir_all(project_dir);
    let _ = fs::remove_dir_all(remote_root);
    let _ = fs::remove_dir_all(cache_dir);
}

#[test]
fn lock_git_branch_rerun_is_up_to_date_and_byte_identical() {
    if !git_available() {
        eprintln!("skipping git lock test: git binary unavailable");
        return;
    }

    let remote_root = temp_dir("lock-git-branch-remote");
    let remote_repo = remote_root.join("remote-lib");
    let _initial_commit = init_git_repo_with_manifest(&remote_repo, "1.0.0");
    git(&remote_repo, &["checkout", "-b", "release"]);
    fs::write(
        remote_repo.join("sysml.toml"),
        "[project]\nname = \"remote-lib\"\nversion = \"1.1.0\"\n",
    )
    .unwrap();
    git(&remote_repo, &["add", "sysml.toml"]);
    git(&remote_repo, &["commit", "-m", "release"]);
    let release_commit = git(&remote_repo, &["rev-parse", "HEAD"]);
    let url = format!("file://{}", remote_repo.canonicalize().unwrap().display());

    let project_dir = temp_dir("lock-git-branch");
    let cache_dir = temp_dir("lock-git-branch-cache");
    fs::write(
        project_dir.join("sysml.toml"),
        format!(
            "[project]\nname = \"root\"\nversion = \"1.0.0\"\n\n[dependencies]\nremote-lib = {{ git = \"{url}\", branch = \"release\" }}\n"
        ),
    )
    .unwrap();

    let first = sysml_bin()
        .arg("lock")
        .env("SYSML_RS_CACHE_DIR", &cache_dir)
        .current_dir(&project_dir)
        .output()
        .expect("failed to run first sysml lock for git branch dependency");
    assert!(
        first.status.success(),
        "first lock should succeed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let lock_path = project_dir.join("sysml.lock");
    let first_bytes = fs::read(&lock_path).unwrap();
    let first_text = String::from_utf8_lossy(&first_bytes);
    assert!(first_text.contains(&format!("git:{url}#{release_commit}")));

    let second = sysml_bin()
        .arg("lock")
        .env("SYSML_RS_CACHE_DIR", &cache_dir)
        .current_dir(&project_dir)
        .output()
        .expect("failed to run second sysml lock for git branch dependency");
    assert!(
        second.status.success(),
        "second lock should succeed: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_stdout = String::from_utf8_lossy(&second.stdout);
    assert!(second_stdout.contains("up to date"));

    let second_bytes = fs::read(&lock_path).unwrap();
    assert_eq!(
        first_bytes, second_bytes,
        "rerunning lock for unchanged git dependency should be byte-identical"
    );

    let _ = fs::remove_dir_all(project_dir);
    let _ = fs::remove_dir_all(remote_root);
    let _ = fs::remove_dir_all(cache_dir);
}

#[test]
fn update_forces_lock_rewrite() {
    let dir = temp_dir("update-lock");
    let dep = dir.join("lib-update");
    fs::create_dir_all(dep.join("src")).unwrap();
    fs::write(
        dep.join("sysml.toml"),
        "[project]\nname = \"lib-update\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(dep.join("src/lib.sysml"), "package LibUpdate {}\n").unwrap();
    fs::write(
        dir.join("sysml.toml"),
        "[project]\nname = \"root\"\nversion = \"0.1.0\"\n\n[dependencies]\nlib-update = { path = \"./lib-update\" }\n",
    )
    .unwrap();

    let first = sysml_bin()
        .arg("lock")
        .current_dir(&dir)
        .output()
        .expect("failed to run initial sysml lock");
    assert!(first.status.success());
    let lock_path = dir.join("sysml.lock");
    let initial_lock = fs::read_to_string(&lock_path).unwrap();
    assert!(initial_lock.contains("0.1.0"));

    fs::write(
        dep.join("sysml.toml"),
        "[project]\nname = \"lib-update\"\nversion = \"0.2.0\"\n",
    )
    .unwrap();

    let output = sysml_bin()
        .arg("update")
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml update");
    assert!(
        output.status.success(),
        "update should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let updated_lock = fs::read_to_string(&lock_path).unwrap();
    assert!(updated_lock.contains("0.2.0"));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn lock_and_update_support_json_and_quiet_output_modes() {
    let dir = temp_dir("lock-update-json-quiet");
    let dep = dir.join("lib-update");
    fs::create_dir_all(dep.join("src")).unwrap();
    fs::write(
        dep.join("sysml.toml"),
        "[project]\nname = \"lib-update\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(dep.join("src/lib.sysml"), "package LibUpdate {}\n").unwrap();
    fs::write(
        dir.join("sysml.toml"),
        "[project]\nname = \"root\"\nversion = \"0.1.0\"\n\n[dependencies]\nlib-update = { path = \"./lib-update\" }\n",
    )
    .unwrap();

    let lock_json = sysml_bin()
        .args(["lock", "--json"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml lock --json");
    assert!(lock_json.status.success());
    let parsed_lock: serde_json::Value = serde_json::from_slice(&lock_json.stdout).unwrap();
    assert_eq!(parsed_lock["status"], "updated");
    let lock_pkgs = parsed_lock["packages"]
        .as_array()
        .expect("lock packages must be an array");
    assert_eq!(lock_pkgs.len(), 1);
    assert_eq!(lock_pkgs[0]["name"], "lib-update");
    assert_eq!(lock_pkgs[0]["version"], "0.1.0");
    assert_eq!(lock_pkgs[0]["source"], "path:./lib-update");
    assert!(
        parsed_lock["lock_path"].as_str().is_some(),
        "lock_path must be serialized as a string"
    );

    let lock_json_second = sysml_bin()
        .args(["lock", "--json"])
        .current_dir(&dir)
        .output()
        .expect("failed to run second sysml lock --json");
    assert!(lock_json_second.status.success());
    let parsed_second: serde_json::Value =
        serde_json::from_slice(&lock_json_second.stdout).unwrap();
    assert_eq!(parsed_second["status"], "up_to_date");
    assert_eq!(parsed_second["packages"], 1);

    let lock_quiet = sysml_bin()
        .args(["lock", "--quiet"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml lock --quiet");
    assert!(lock_quiet.status.success());
    assert!(
        String::from_utf8_lossy(&lock_quiet.stdout)
            .trim()
            .is_empty(),
        "lock --quiet should suppress stdout"
    );

    fs::write(
        dep.join("sysml.toml"),
        "[project]\nname = \"lib-update\"\nversion = \"0.2.0\"\n",
    )
    .unwrap();

    let update_json = sysml_bin()
        .args(["update", "--json"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml update --json");
    assert!(update_json.status.success());
    let parsed_update: serde_json::Value = serde_json::from_slice(&update_json.stdout).unwrap();
    assert_eq!(parsed_update["status"], "updated");
    let update_pkgs = parsed_update["packages"]
        .as_array()
        .expect("update packages must be an array");
    assert_eq!(update_pkgs.len(), 1);
    assert_eq!(update_pkgs[0]["name"], "lib-update");
    assert_eq!(update_pkgs[0]["version"], "0.2.0");

    let update_quiet = sysml_bin()
        .args(["update", "--quiet"])
        .current_dir(&dir)
        .output()
        .expect("failed to run sysml update --quiet");
    assert!(update_quiet.status.success());
    assert!(
        String::from_utf8_lossy(&update_quiet.stdout)
            .trim()
            .is_empty(),
        "update --quiet should suppress stdout"
    );

    let _ = fs::remove_dir_all(dir);
}
