use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use directories::{BaseDirs, ProjectDirs};
use sha2::{Digest, Sha256};
use sysml_manifest::{load_lock, load_manifest, save_lock, LOCK_FILENAME};
use sysml_resolve::{generate_lock, is_lock_up_to_date, resolve, PackageSource};
use tempfile::TempDir;

#[test]
fn resolve_git_rev_tag_and_branch_from_local_repo() {
    if !git_available() {
        eprintln!("skipping git integration test: git binary unavailable");
        return;
    }

    let fixture = create_git_fixture();
    clean_git_cache(&fixture.url);

    git(&fixture.repo_dir, &["tag", "v1.0.0"]);
    git(&fixture.repo_dir, &["checkout", "-b", "release"]);
    fs::write(
        fixture.repo_dir.join("sysml.toml"),
        "[project]\nname = \"remote-lib\"\nversion = \"1.1.0\"\n",
    )
    .unwrap();
    git(&fixture.repo_dir, &["add", "sysml.toml"]);
    git(&fixture.repo_dir, &["commit", "-m", "release"]);
    let release_commit = git(&fixture.repo_dir, &["rev-parse", "HEAD"]);

    assert_resolved_commit(
        fixture.tmp.path(),
        &fixture.url,
        &format!("rev = \"{}\"", fixture.main_commit),
        &fixture.main_commit,
    );
    assert_resolved_commit(
        fixture.tmp.path(),
        &fixture.url,
        "tag = \"v1.0.0\"",
        &fixture.main_commit,
    );
    assert_resolved_commit(
        fixture.tmp.path(),
        &fixture.url,
        "branch = \"release\"",
        &release_commit,
    );

    clean_git_cache(&fixture.url);
}

#[test]
fn git_lock_generation_is_idempotent() {
    if !git_available() {
        eprintln!("skipping git integration test: git binary unavailable");
        return;
    }

    let fixture = create_git_fixture();
    clean_git_cache(&fixture.url);
    let root_dir = fixture.tmp.path().join("idempotent-root");
    fs::create_dir_all(&root_dir).unwrap();

    write_root_manifest(
        &root_dir,
        &fixture.url,
        &format!("rev = \"{}\"", fixture.main_commit),
    );

    let manifest = load_manifest(&root_dir.join("sysml.toml")).unwrap();

    let graph_1 = resolve(&manifest, &root_dir).unwrap();
    let lock_1 = generate_lock(&graph_1);
    let lock_path = root_dir.join(LOCK_FILENAME);
    save_lock(&lock_path, &lock_1).unwrap();
    let bytes_1 = fs::read(&lock_path).unwrap();

    let graph_2 = resolve(&manifest, &root_dir).unwrap();
    let lock_2 = generate_lock(&graph_2);
    save_lock(&lock_path, &lock_2).unwrap();
    let bytes_2 = fs::read(&lock_path).unwrap();

    assert_eq!(bytes_1, bytes_2, "git lock output should be deterministic");

    let loaded = load_lock(&lock_path).unwrap();
    assert!(is_lock_up_to_date(&graph_2, &loaded));

    clean_git_cache(&fixture.url);
}

fn assert_resolved_commit(base: &Path, url: &str, ref_line: &str, expected_commit: &str) {
    let root = base.join(format!(
        "root-{}",
        expected_commit.chars().take(8).collect::<String>()
    ));
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(&root).unwrap();

    write_root_manifest(&root, url, ref_line);

    let manifest = load_manifest(&root.join("sysml.toml")).unwrap();
    let graph = resolve(&manifest, &root).unwrap();

    assert_eq!(graph.len(), 1);
    let pkg = &graph.packages[0];
    assert_eq!(pkg.name, "remote-lib");
    assert_eq!(
        pkg.source,
        PackageSource::Git {
            url: url.to_string(),
            commit: expected_commit.to_string(),
        }
    );
}

fn write_root_manifest(root: &Path, url: &str, ref_line: &str) {
    fs::write(
        root.join("sysml.toml"),
        format!(
            "[project]\nname = \"root\"\nversion = \"0.1.0\"\n\n[dependencies]\nremote-lib = {{ git = \"{url}\", {ref_line} }}\n"
        ),
    )
    .unwrap();
}

struct GitFixture {
    tmp: TempDir,
    repo_dir: PathBuf,
    url: String,
    main_commit: String,
}

fn create_git_fixture() -> GitFixture {
    let tmp = TempDir::new().unwrap();
    let repo_dir = tmp.path().join("remote-lib");
    fs::create_dir_all(&repo_dir).unwrap();

    git(&repo_dir, &["init", "--initial-branch", "main"]);
    git(
        &repo_dir,
        &["config", "user.email", "sysml-tests@example.com"],
    );
    git(&repo_dir, &["config", "user.name", "SysML Tests"]);

    fs::write(
        repo_dir.join("sysml.toml"),
        "[project]\nname = \"remote-lib\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();

    git(&repo_dir, &["add", "sysml.toml"]);
    git(&repo_dir, &["commit", "-m", "initial"]);
    let main_commit = git(&repo_dir, &["rev-parse", "HEAD"]);

    let canonical = repo_dir.canonicalize().unwrap();
    let url = format!("file://{}", canonical.display());

    GitFixture {
        tmp,
        repo_dir,
        url,
        main_commit,
    }
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

fn clean_git_cache(url: &str) {
    let cache_dir = cache_root()
        .join("dependencies")
        .join("git")
        .join(source_hash(url));
    let _ = fs::remove_dir_all(cache_dir);
}

fn cache_root() -> PathBuf {
    if let Some(project_dirs) = ProjectDirs::from("", "", "sysml-rs") {
        return project_dirs.cache_dir().to_path_buf();
    }

    if let Some(base_dirs) = BaseDirs::new() {
        return base_dirs.cache_dir().join("sysml-rs");
    }

    PathBuf::from("/tmp/sysml-rs-cache")
}

fn source_hash(source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    format!("{:x}", hasher.finalize())
}
