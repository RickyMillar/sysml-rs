use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};
use sysml_project::kpar::{write_kpar, KparArchive, ProjectInfo, ProjectMetadata};
use sysml_manifest::{load_manifest, save_lock, LOCK_FILENAME};
use sysml_resolve::{generate_lock, resolve, ResolveError};
use tempfile::TempDir;

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lock")
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if file_type.is_file() {
            fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

fn stage_fixture(fixture_id: &str) -> (TempDir, PathBuf) {
    let fixture_src = fixtures_root().join(fixture_id);
    assert!(
        fixture_src.exists(),
        "fixture source directory does not exist: {}",
        fixture_src.display()
    );

    let tmp = TempDir::new().expect("failed to create temp dir for fixture");
    let fixture_dst = tmp.path().join(fixture_id);
    copy_dir_recursive(&fixture_src, &fixture_dst)
        .expect("failed to copy fixture into temporary directory");

    (tmp, fixture_dst)
}

fn prepare_fixture_artifacts(fixture_id: &str, fixture_dir: &Path) -> BTreeMap<String, String> {
    let mut replacements = BTreeMap::new();
    match fixture_id {
        "fx09_kpar_file" | "fx10_kpar_checksum_mismatch" => {
            let archive_path = fixture_dir.join("deps/archive-lib.kpar");
            fs::create_dir_all(archive_path.parent().unwrap()).unwrap();
            write_fixture_kpar(&archive_path, "archive-lib", "1.0.0");
            replacements.insert(
                "{{CHECKSUM}}".to_string(),
                format!("sha256:{}", sha256_hex_file(&archive_path)),
            );
        }
        "fx11_registry_exact" => {
            let index_dir = fixture_dir.join("root/.sysml/registries/sysand");
            fs::create_dir_all(index_dir.join("artifacts")).unwrap();
            let archive_path = index_dir.join("artifacts/units-1.4.2.kpar");
            write_fixture_kpar(&archive_path, "units", "1.4.2");
            let checksum = format!("sha256:{}", sha256_hex_file(&archive_path));
            fs::write(
                index_dir.join("index.json"),
                format!(
                    "{{\"packages\":{{\"units\":{{\"1.4.2\":{{\"artifact\":\"artifacts/units-1.4.2.kpar\",\"checksum\":\"{checksum}\"}}}}}}}}"
                ),
            )
            .unwrap();
            clean_registry_cache_for_request("sysand", "units", "1.4.2");
            replacements.insert("{{CHECKSUM}}".to_string(), checksum);
        }
        "fx13_registry_range" => {
            let index_dir = fixture_dir.join("root/.sysml/registries/sysand");
            fs::create_dir_all(index_dir.join("artifacts")).unwrap();

            let releases = ["1.4.2", "1.9.3", "2.0.0"];
            let mut entries = Vec::new();
            let mut selected_checksum = String::new();
            for version in releases {
                let archive_path = index_dir.join(format!("artifacts/units-{version}.kpar"));
                write_fixture_kpar(&archive_path, "units", version);
                let checksum = format!("sha256:{}", sha256_hex_file(&archive_path));
                if version == "1.9.3" {
                    selected_checksum = checksum.clone();
                }
                entries.push(format!(
                    "\"{version}\":{{\"artifact\":\"artifacts/units-{version}.kpar\",\"checksum\":\"{checksum}\"}}"
                ));
            }
            fs::write(
                index_dir.join("index.json"),
                format!("{{\"packages\":{{\"units\":{{{}}}}}}}", entries.join(",")),
            )
            .unwrap();
            clean_registry_cache_for_request("sysand", "units", "^1.4");
            replacements.insert("{{CHECKSUM}}".to_string(), selected_checksum);
        }
        "fx12_mixed_sources" => {
            let root_dir = fixture_dir.join("root");
            let deps_dir = fixture_dir.join("deps");

            let dep_kpar_archive = deps_dir.join("dep-kpar.kpar");
            write_fixture_kpar(&dep_kpar_archive, "dep-kpar", "3.0.0");
            let dep_kpar_checksum = format!("sha256:{}", sha256_hex_file(&dep_kpar_archive));
            clean_kpar_cache_for_source(&dep_kpar_archive);

            let index_dir = root_dir.join(".sysml/registries/sysand");
            fs::create_dir_all(index_dir.join("artifacts")).unwrap();
            let dep_reg_archive = index_dir.join("artifacts/dep-reg-4.5.6.kpar");
            write_fixture_kpar(&dep_reg_archive, "dep-reg", "4.5.6");
            let dep_reg_checksum = format!("sha256:{}", sha256_hex_file(&dep_reg_archive));
            fs::write(
                index_dir.join("index.json"),
                format!(
                    "{{\"packages\":{{\"dep-reg\":{{\"4.5.6\":{{\"artifact\":\"artifacts/dep-reg-4.5.6.kpar\",\"checksum\":\"{dep_reg_checksum}\"}}}}}}}}"
                ),
            )
            .unwrap();
            clean_registry_cache_for_request("sysand", "dep-reg", "4.5.6");

            let dep_git_dir = deps_dir.join("dep-git");
            git(&dep_git_dir, &["init", "--initial-branch", "main"]);
            git(
                &dep_git_dir,
                &["config", "user.email", "sysml-tests@example.com"],
            );
            git(&dep_git_dir, &["config", "user.name", "SysML Tests"]);
            git(&dep_git_dir, &["add", "sysml.toml"]);
            git(&dep_git_dir, &["commit", "-m", "initial"]);
            let dep_git_commit = git(&dep_git_dir, &["rev-parse", "HEAD"]);
            let dep_git_url = format!("file://{}", dep_git_dir.canonicalize().unwrap().display());
            clean_git_cache(&dep_git_url);

            let manifest_path = root_dir.join("sysml.toml");
            let manifest = fs::read_to_string(&manifest_path)
                .unwrap()
                .replace("{{GIT_URL}}", &dep_git_url)
                .replace("{{GIT_COMMIT}}", &dep_git_commit);
            fs::write(&manifest_path, manifest).unwrap();

            replacements.insert("{{GIT_URL}}".to_string(), dep_git_url);
            replacements.insert("{{GIT_COMMIT}}".to_string(), dep_git_commit);
            replacements.insert("{{KPAR_CHECKSUM}}".to_string(), dep_kpar_checksum);
            replacements.insert("{{REGISTRY_CHECKSUM}}".to_string(), dep_reg_checksum);
        }
        _ => {}
    }

    replacements
}

fn generate_lock_bytes(root_dir: &Path) -> Vec<u8> {
    let manifest_path = root_dir.join("sysml.toml");
    let manifest = load_manifest(&manifest_path).expect("failed to load fixture manifest");

    let graph = resolve(&manifest, root_dir).expect("fixture dependency resolution should succeed");
    let lock = generate_lock(&graph);

    let lock_path = root_dir.join(LOCK_FILENAME);
    save_lock(&lock_path, &lock).expect("failed to write generated sysml.lock");

    fs::read(&lock_path).expect("failed to read generated sysml.lock")
}

fn assert_success_fixture(fixture_id: &str) {
    let (_tmp, fixture_dir) = stage_fixture(fixture_id);
    let replacements = prepare_fixture_artifacts(fixture_id, &fixture_dir);

    let root_dir = fixture_dir.join("root");
    let expected_path = fixture_dir.join("expected.sysml.lock");
    let lock_path = root_dir.join(LOCK_FILENAME);

    let expected_template =
        fs::read_to_string(&expected_path).expect("failed to read expected.sysml.lock for fixture");
    let mut expected = expected_template;
    for (token, value) in replacements {
        expected = expected.replace(&token, &value);
    }
    let expected_bytes = expected.into_bytes();

    assert!(
        !lock_path.exists(),
        "fixture root should not include a pre-existing lock file: {}",
        lock_path.display()
    );

    let first = generate_lock_bytes(&root_dir);
    assert_eq!(
        first, expected_bytes,
        "generated lock does not match expected bytes for fixture '{fixture_id}'"
    );

    let second = generate_lock_bytes(&root_dir);
    assert_eq!(
        second, expected_bytes,
        "second generated lock does not match expected bytes for fixture '{fixture_id}'"
    );
    assert_eq!(
        first, second,
        "lock generation is non-deterministic for fixture '{fixture_id}'"
    );
}

#[test]
fn lock_fixtures_match_expected_and_are_deterministic() {
    if !git_available() {
        eprintln!("skipping fx12_mixed_sources fixture: git binary unavailable");
    }

    for fixture_id in [
        "fx01_path_single",
        "fx02_path_transitive",
        "fx03_path_diamond",
        "fx09_kpar_file",
        "fx11_registry_exact",
        "fx13_registry_range",
    ] {
        assert_success_fixture(fixture_id);
    }

    if git_available() {
        assert_success_fixture("fx12_mixed_sources");
    }
}

#[test]
fn lock_fixture_cycle_errors_and_does_not_write_lock() {
    let (_tmp, fixture_dir) = stage_fixture("fx04_path_cycle");

    let root_dir = fixture_dir.join("root");
    let lock_path = root_dir.join(LOCK_FILENAME);

    assert!(
        !lock_path.exists(),
        "fixture root should not include a pre-existing lock file: {}",
        lock_path.display()
    );

    let manifest_path = root_dir.join("sysml.toml");
    let manifest = load_manifest(&manifest_path).expect("failed to load cycle fixture manifest");

    let result = resolve(&manifest, &root_dir);
    assert!(
        matches!(result, Err(ResolveError::Cycle { .. })),
        "expected cycle error, got: {result:?}"
    );

    assert!(
        !lock_path.exists(),
        "sysml.lock should not be written for cycle fixture: {}",
        lock_path.display()
    );
}

#[test]
fn lock_fixture_kpar_checksum_mismatch_errors_and_does_not_write_lock() {
    let (_tmp, fixture_dir) = stage_fixture("fx10_kpar_checksum_mismatch");
    prepare_fixture_artifacts("fx10_kpar_checksum_mismatch", &fixture_dir);

    let root_dir = fixture_dir.join("root");
    let lock_path = root_dir.join(LOCK_FILENAME);
    let source_archive = fixture_dir.join("deps/archive-lib.kpar");
    clean_kpar_cache_for_source(&source_archive);

    let manifest_path = root_dir.join("sysml.toml");
    let manifest = load_manifest(&manifest_path).expect("failed to load checksum fixture manifest");

    let first = resolve(&manifest, &root_dir).expect("first resolution should populate cache");
    assert_eq!(first.len(), 1);

    let canonical = source_archive.canonicalize().unwrap();
    let source_key = canonical_file_source(&canonical);
    let expected_hex = sha256_hex_file(&canonical);
    let cached_archive = kpar_cache_dir_for_source(&source_key)
        .join("archives")
        .join(format!("{expected_hex}.kpar"));
    assert!(
        cached_archive.exists(),
        "expected cached archive at {}",
        cached_archive.display()
    );
    fs::write(&cached_archive, b"corrupt-cache-archive").unwrap();

    let err = resolve(&manifest, &root_dir).unwrap_err();
    assert!(
        matches!(&err, ResolveError::ChecksumMismatch { .. }),
        "expected checksum mismatch error, got: {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("checksum mismatch") && msg.contains("remove cache"),
        "expected actionable checksum mismatch message, got: {msg}"
    );

    assert!(
        !lock_path.exists(),
        "sysml.lock should not be written for checksum mismatch fixture: {}",
        lock_path.display()
    );

    clean_kpar_cache_for_source(&source_archive);
}

#[test]
fn lock_fixture_registry_checksum_mismatch_errors_and_does_not_write_lock() {
    let (_tmp, fixture_dir) = stage_fixture("fx11_registry_exact");
    prepare_fixture_artifacts("fx11_registry_exact", &fixture_dir);

    let root_dir = fixture_dir.join("root");
    let lock_path = root_dir.join(LOCK_FILENAME);
    clean_registry_cache_for_request("sysand", "units", "1.4.2");

    let manifest_path = root_dir.join("sysml.toml");
    let manifest = load_manifest(&manifest_path).expect("failed to load registry fixture manifest");

    let first = resolve(&manifest, &root_dir).expect("first resolution should populate cache");
    assert_eq!(first.len(), 1);

    let source_archive =
        fixture_dir.join("root/.sysml/registries/sysand/artifacts/units-1.4.2.kpar");
    let expected_hex = sha256_hex_file(&source_archive);
    let cached_archive = registry_cache_dir_for_request("sysand", "units", "1.4.2")
        .join("artifacts")
        .join(format!("{expected_hex}.kpar"));
    assert!(
        cached_archive.exists(),
        "expected cached archive at {}",
        cached_archive.display()
    );

    fs::write(&cached_archive, b"corrupt-registry-cache").unwrap();

    let err = resolve(&manifest, &root_dir).unwrap_err();
    assert!(
        matches!(&err, ResolveError::ChecksumMismatch { .. }),
        "expected checksum mismatch error, got: {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("checksum mismatch") && msg.contains("remove cache"),
        "expected actionable checksum mismatch message, got: {msg}"
    );
    assert!(
        !lock_path.exists(),
        "sysml.lock should not be written for registry checksum mismatch fixture: {}",
        lock_path.display()
    );

    clean_registry_cache_for_request("sysand", "units", "1.4.2");
}

#[test]
fn lock_fixture_registry_range_no_match_errors_and_does_not_write_lock() {
    let (_tmp, fixture_dir) = stage_fixture("fx14_registry_range_no_match");

    let root_dir = fixture_dir.join("root");
    let lock_path = root_dir.join(LOCK_FILENAME);
    let index_dir = root_dir.join(".sysml/registries/sysand");
    fs::create_dir_all(index_dir.join("artifacts")).unwrap();

    let archive_path = index_dir.join("artifacts/units-1.4.2.kpar");
    write_fixture_kpar(&archive_path, "units", "1.4.2");
    let checksum = format!("sha256:{}", sha256_hex_file(&archive_path));
    fs::write(
        index_dir.join("index.json"),
        format!(
            "{{\"packages\":{{\"units\":{{\"1.4.2\":{{\"artifact\":\"artifacts/units-1.4.2.kpar\",\"checksum\":\"{checksum}\"}}}}}}}}"
        ),
    )
    .unwrap();
    clean_registry_cache_for_request("sysand", "units", "~3.1");

    let manifest_path = root_dir.join("sysml.toml");
    let manifest = load_manifest(&manifest_path).expect("failed to load range fixture manifest");

    let err = resolve(&manifest, &root_dir).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("no compatible release") && msg.contains("~3.1"),
        "expected actionable no-compatible-release message, got: {msg}"
    );
    assert!(
        !lock_path.exists(),
        "sysml.lock should not be written for registry no-match fixture: {}",
        lock_path.display()
    );

    clean_registry_cache_for_request("sysand", "units", "~3.1");
}

fn write_fixture_kpar(path: &Path, name: &str, version: &str) {
    let mut metadata = ProjectMetadata::new();
    metadata.created = Some("2026-03-01T00:00:00Z".to_string());
    metadata.add_index_entry("ArchiveRoot", "ArchiveRoot.sysml");

    let archive = KparArchive {
        root_dir: name.to_string(),
        project_info: ProjectInfo {
            name: name.to_string(),
            version: version.to_string(),
            description: Some("fixture archive".to_string()),
            license: Some("MIT".to_string()),
            usage: Vec::new(),
        },
        metadata,
        source_files: vec![(
            "ArchiveRoot.sysml".to_string(),
            b"package ArchiveRoot {\n  part def Sensor;\n}\n".to_vec(),
        )],
    };

    write_kpar(path, &archive).unwrap();
}

fn clean_kpar_cache_for_source(source_archive: &Path) {
    let canonical = source_archive.canonicalize().unwrap();
    let source_key = canonical_file_source(&canonical);
    let cache_dir = kpar_cache_dir_for_source(&source_key);
    let _ = fs::remove_dir_all(cache_dir);
}

fn kpar_cache_dir_for_source(source_key: &str) -> PathBuf {
    cache_root()
        .join("dependencies")
        .join("kpar")
        .join(source_hash(source_key))
}

fn registry_cache_dir_for_request(backend: &str, package: &str, requirement: &str) -> PathBuf {
    let request_key = format!("{backend}:{package}@{requirement}");
    cache_root()
        .join("dependencies")
        .join("registry")
        .join(backend)
        .join(source_hash(&request_key))
}

fn clean_git_cache(url: &str) {
    let cache_dir = cache_root()
        .join("dependencies")
        .join("git")
        .join(source_hash(url));
    let _ = fs::remove_dir_all(cache_dir);
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

fn canonical_file_source(path: &Path) -> String {
    format!("file://{}", path.display())
}

fn sha256_hex_file(path: &Path) -> String {
    let bytes = fs::read(path).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn source_hash(source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn clean_registry_cache_for_request(backend: &str, package: &str, requirement: &str) {
    let cache_dir = registry_cache_dir_for_request(backend, package, requirement);
    let _ = fs::remove_dir_all(cache_dir);
}

fn cache_root() -> PathBuf {
    if let Some(project_dirs) = directories::ProjectDirs::from("", "", "sysml-rs") {
        return project_dirs.cache_dir().to_path_buf();
    }

    if let Some(base_dirs) = directories::BaseDirs::new() {
        return base_dirs.cache_dir().join("sysml-rs");
    }

    PathBuf::from("/tmp/sysml-rs-cache")
}
