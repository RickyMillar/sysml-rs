use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use sha2::{Digest, Sha256};
use sysml_project::kpar::{write_kpar, KparArchive, ProjectInfo, ProjectMetadata};

fn sysml_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sysml"))
}

fn temp_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "sysml-cli-inspect-test-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("create temp dir");
    path
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {}: {e}", args.join(" ")));

    assert!(
        output.status.success(),
        "git {} failed in {}\nstdout: {}\nstderr: {}",
        args.join(" "),
        cwd.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn init_git_repo_with_model(
    repo_dir: &Path,
    name: &str,
    version: &str,
    package_name: &str,
    type_name: &str,
) -> (String, String) {
    fs::create_dir_all(repo_dir).unwrap();
    git(repo_dir, &["init", "--initial-branch", "main"]);
    git(
        repo_dir,
        &["config", "user.email", "sysml-tests@example.com"],
    );
    git(repo_dir, &["config", "user.name", "SysML Tests"]);
    fs::write(
        repo_dir.join("sysml.toml"),
        format!(
            "[project]\nname = \"{name}\"\nversion = \"{version}\"\nsysml-edition = \"2025\"\n"
        ),
    )
    .unwrap();
    fs::write(
        repo_dir.join("dep.sysml"),
        format!("package {package_name} {{ part def {type_name}; }}\n"),
    )
    .unwrap();
    git(repo_dir, &["add", "sysml.toml", "dep.sysml"]);
    git(repo_dir, &["commit", "-m", "initial"]);
    let commit = git(repo_dir, &["rev-parse", "HEAD"]);
    (format!("file://{}", repo_dir.display()), commit)
}

fn create_kpar_archive_with_model(
    root: &Path,
    name: &str,
    version: &str,
    package_name: &str,
    type_name: &str,
) -> PathBuf {
    let mut metadata = ProjectMetadata::new();
    metadata.add_index_entry("Root", "Root.sysml");
    let archive = KparArchive {
        root_dir: name.to_string(),
        project_info: ProjectInfo {
            name: name.to_string(),
            version: version.to_string(),
            description: Some("inspect integration fixture".to_string()),
            license: Some("MIT".to_string()),
            usage: Vec::new(),
        },
        metadata,
        source_files: vec![(
            "Root.sysml".to_string(),
            format!("package {package_name} {{ part def {type_name}; }}\n").into_bytes(),
        )],
    };
    let archive_path = root.join(format!("{name}-{version}.kpar"));
    write_kpar(&archive_path, &archive).expect("kpar fixture should be written");
    archive_path
}

fn sha256_hex_file(path: &Path) -> String {
    let bytes = fs::read(path).expect("artifact should be readable");
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn write_sysand_index(root: &Path, package: &str, version: &str, artifact_path: &Path) {
    let index_dir = root.join(".sysml/registries/sysand");
    fs::create_dir_all(index_dir.join("artifacts")).expect("index artifacts dir should exist");
    let checksum = format!("sha256:{}", sha256_hex_file(artifact_path));
    fs::write(
        index_dir.join("index.json"),
        format!(
            "{{\"packages\":{{\"{package}\":{{\"{version}\":{{\"artifact\":\"{}\",\"checksum\":\"{checksum}\"}}}}}}}}",
            artifact_path.display()
        ),
    )
    .expect("sysand index should be written");
}

struct InspectWorkspaceFixture {
    root: PathBuf,
    workspace: PathBuf,
    manifest_path: PathBuf,
    focus_file_name: String,
    git_commit: String,
}

fn create_fixture() -> InspectWorkspaceFixture {
    let root = temp_dir("workspace-deps");
    let workspace = root.join("ws");
    fs::create_dir_all(&workspace).unwrap();

    let path_dep = root.join("dep-path");
    fs::create_dir_all(&path_dep).unwrap();
    fs::write(
        path_dep.join("sysml.toml"),
        "[project]\nname = \"path-lib\"\nversion = \"0.1.0\"\nsysml-edition = \"2025\"\n",
    )
    .unwrap();
    fs::write(
        path_dep.join("path-lib.sysml"),
        "package PathLib { part def PathSensor; }\n",
    )
    .unwrap();

    let git_repo = root.join("dep-git");
    let (git_url, git_commit) =
        init_git_repo_with_model(&git_repo, "git-lib", "0.1.0", "GitLib", "GitController");

    let kpar_archive =
        create_kpar_archive_with_model(&root, "kpar-lib", "0.1.0", "KparLib", "KparActuator");
    let registry_archive = create_kpar_archive_with_model(
        &root,
        "registry-lib",
        "1.0.0",
        "RegistryLib",
        "RegistryBus",
    );
    write_sysand_index(&workspace, "registry-lib", "1.0.0", &registry_archive);

    let manifest_path = workspace.join("sysml.toml");
    fs::write(
        &manifest_path,
        format!(
            r#"[project]
name = "inspect-workspace"
version = "0.1.0"
sysml-edition = "2025"

[dependencies]
path-ok = {{ path = "../dep-path" }}
git-ok = {{ git = "{git_url}", rev = "{git_commit}" }}
kpar-ok = {{ kpar = "{}" }}
registry-lib = "1.0.0"
"#,
            kpar_archive.display()
        ),
    )
    .unwrap();

    let model_dir = workspace.join("model");
    fs::create_dir_all(&model_dir).unwrap();
    let focus_file_name = "main.sysml".to_string();
    fs::write(
        model_dir.join(&focus_file_name),
        r#"package WorkspaceInspect {
    import PathLib::*;
    import GitLib::*;
    import KparLib::*;
    import RegistryLib::*;

    part def Rig {
        part pathSensor : PathSensor;
        part gitController : GitController;
        part kparActuator : KparActuator;
        part registryBus : RegistryBus;
    }
}
"#,
    )
    .unwrap();

    InspectWorkspaceFixture {
        root,
        workspace,
        manifest_path,
        focus_file_name,
        git_commit,
    }
}

fn run_inspect_workspace(workspace: &Path, focus_file_name: &str, extra_args: &[&str]) -> Output {
    let mut cmd = sysml_bin();
    cmd.arg("inspect")
        .arg("--workspace")
        .arg(workspace)
        .arg("--focus")
        .arg(focus_file_name)
        .arg("--diagnostics")
        .arg("--json");
    for arg in extra_args {
        cmd.arg(arg);
    }
    cmd.current_dir(workspace)
        .output()
        .expect("failed to run sysml inspect --workspace")
}

fn diagnostics_for_focus(stdout: &[u8], focus_file_name: &str) -> Vec<String> {
    let parsed: Value =
        serde_json::from_slice(stdout).expect("inspect output should be valid JSON");
    let files = parsed["files"]
        .as_array()
        .expect("inspect json should contain files array");
    let focus_entry = files
        .iter()
        .find(|entry| {
            entry["file"]
                .as_str()
                .map(|path| {
                    path.ends_with(focus_file_name)
                        || Path::new(path).file_name().and_then(|name| name.to_str())
                            == Some(focus_file_name)
                })
                .unwrap_or(false)
        })
        .unwrap_or_else(|| panic!("missing focused file entry for {}", focus_file_name));
    focus_entry["diagnostics"]
        .as_array()
        .expect("inspect json file entry should contain diagnostics array")
        .iter()
        .filter_map(|diag| diag["message"].as_str().map(|s| s.to_string()))
        .collect()
}

fn unresolved_messages(messages: &[String]) -> Vec<&String> {
    messages
        .iter()
        .filter(|msg| {
            msg.contains("import references namespace")
                || (msg.contains("no definition '") && msg.contains("found in scope"))
        })
        .collect()
}

#[test]
fn inspect_workspace_resolves_all_dependency_sources_by_default() {
    if !git_available() {
        eprintln!("skipping inspect workspace dependency test: git binary unavailable");
        return;
    }

    let fixture = create_fixture();
    let output = run_inspect_workspace(&fixture.workspace, &fixture.focus_file_name, &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "inspect workspace should succeed with dependency hydration\nstderr:\n{}",
        stderr
    );

    let messages = diagnostics_for_focus(&output.stdout, &fixture.focus_file_name);
    let unresolved = unresolved_messages(&messages);
    assert!(
        unresolved.is_empty(),
        "expected no unresolved import/type diagnostics with workspace deps enabled, got: {:?}",
        unresolved
    );

    let _ = fs::remove_dir_all(&fixture.root);
}

#[test]
fn inspect_workspace_can_disable_dependency_sources_explicitly() {
    if !git_available() {
        eprintln!("skipping inspect workspace dependency test: git binary unavailable");
        return;
    }

    let fixture = create_fixture();
    let output = run_inspect_workspace(
        &fixture.workspace,
        &fixture.focus_file_name,
        &["--no-workspace-deps"],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "inspect workspace should still succeed when dependency hydration is disabled\nstderr:\n{}",
        stderr
    );

    let messages = diagnostics_for_focus(&output.stdout, &fixture.focus_file_name);
    let unresolved = unresolved_messages(&messages);
    assert!(
        !unresolved.is_empty(),
        "expected unresolved import/type diagnostics when --no-workspace-deps is set"
    );

    let _ = fs::remove_dir_all(&fixture.root);
}

#[test]
fn inspect_workspace_continues_when_dependency_resolution_fails() {
    if !git_available() {
        eprintln!("skipping inspect workspace dependency test: git binary unavailable");
        return;
    }

    let fixture = create_fixture();
    let manifest_content = fs::read_to_string(&fixture.manifest_path).unwrap();
    let broken_manifest = manifest_content.replace(&fixture.git_commit, "deadbeef");
    fs::write(&fixture.manifest_path, broken_manifest).unwrap();

    let output = run_inspect_workspace(&fixture.workspace, &fixture.focus_file_name, &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "inspect workspace should continue even when dependency resolution fails\nstderr:\n{}",
        stderr
    );
    assert!(
        stderr.contains("workspace dependencies: resolution failed"),
        "expected dependency resolution failure note in stderr, got:\n{}",
        stderr
    );

    let messages = diagnostics_for_focus(&output.stdout, &fixture.focus_file_name);
    let unresolved = unresolved_messages(&messages);
    assert!(
        !unresolved.is_empty(),
        "expected unresolved import/type diagnostics after dependency resolution failure"
    );

    let _ = fs::remove_dir_all(&fixture.root);
}
