//! Shared helpers for project management integration tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use directories::{BaseDirs, ProjectDirs};
use sha2::{Digest, Sha256};
use sysml_project::kpar::{write_kpar, KparArchive, ProjectInfo, ProjectMetadata};

pub fn sysml_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sysml"))
}

pub fn temp_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "sysml-cli-project-test-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("create temp dir");
    path
}

pub fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(relative)
}

pub fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

pub fn git(cwd: &Path, args: &[&str]) -> String {
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

pub fn init_git_repo_with_manifest(repo_dir: &Path, version: &str) -> String {
    fs::create_dir_all(repo_dir).unwrap();
    git(repo_dir, &["init", "--initial-branch", "main"]);
    git(
        repo_dir,
        &["config", "user.email", "sysml-tests@example.com"],
    );
    git(repo_dir, &["config", "user.name", "SysML Tests"]);
    fs::write(
        repo_dir.join("sysml.toml"),
        format!("[project]\nname = \"remote-lib\"\nversion = \"{version}\"\n"),
    )
    .unwrap();
    git(repo_dir, &["add", "sysml.toml"]);
    git(repo_dir, &["commit", "-m", "initial"]);
    git(repo_dir, &["rev-parse", "HEAD"])
}

pub fn write_fixture_kpar(path: &Path, name: &str, version: &str) {
    let mut metadata = ProjectMetadata::new();
    metadata.created = Some("2026-03-01T00:00:00Z".to_string());
    metadata.add_index_entry("Root", "Root.sysml");

    let archive = KparArchive {
        root_dir: name.to_string(),
        project_info: ProjectInfo {
            name: name.to_string(),
            version: version.to_string(),
            description: Some("cli fixture archive".to_string()),
            license: Some("MIT".to_string()),
            usage: Vec::new(),
        },
        metadata,
        source_files: vec![(
            "Root.sysml".to_string(),
            b"package Root {\n  part def Unit;\n}\n".to_vec(),
        )],
    };

    write_kpar(path, &archive).unwrap();
}

pub fn sha256_hex_file(path: &Path) -> String {
    let bytes = fs::read(path).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn cache_root() -> PathBuf {
    if let Some(project_dirs) = ProjectDirs::from("", "", "sysml-rs") {
        return project_dirs.cache_dir().to_path_buf();
    }

    if let Some(base_dirs) = BaseDirs::new() {
        return base_dirs.cache_dir().join("sysml-rs");
    }

    PathBuf::from("/tmp/sysml-rs-cache")
}

pub fn source_hash(source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn clean_registry_cache_for_request(backend: &str, package: &str, requirement: &str) {
    let request_key = format!("{backend}:{package}@{requirement}");
    let cache_dir = cache_root()
        .join("dependencies")
        .join("registry")
        .join(backend)
        .join(source_hash(&request_key));
    let _ = fs::remove_dir_all(cache_dir);
}

pub fn write_sysand_index_with_releases(root: &Path, package: &str, releases: &[(&str, PathBuf)]) {
    let index_dir = root.join(".sysml/registries/sysand");
    fs::create_dir_all(index_dir.join("artifacts")).unwrap();
    let entries = releases
        .iter()
        .map(|(version, artifact)| {
            let checksum = format!("sha256:{}", sha256_hex_file(artifact));
            format!(
                "\"{version}\":{{\"artifact\":\"artifacts/{package}-{version}.kpar\",\"checksum\":\"{checksum}\"}}"
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    fs::write(
        index_dir.join("index.json"),
        format!("{{\"packages\":{{\"{package}\":{{{entries}}}}}}}"),
    )
    .unwrap();
}

pub fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path);
        } else {
            fs::copy(&src_path, &dst_path).unwrap();
        }
    }
}
