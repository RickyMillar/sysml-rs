use std::fs;
use std::path::{Path, PathBuf};

use directories::{BaseDirs, ProjectDirs};
use sha2::{Digest, Sha256};
use sysml_project::kpar::{write_kpar, KparArchive, ProjectInfo, ProjectMetadata};
use sysml_manifest::{load_lock, load_manifest, save_lock, LOCK_FILENAME};
use sysml_resolve::{generate_lock, is_lock_up_to_date, resolve, PackageSource, ResolveError};
use tempfile::TempDir;

#[test]
fn registry_exact_version_lock_generation_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let package = "units-registry-it-a";
    let version = "2.3.4";
    let archive_path = setup_registry_fixture(root, package, version);
    clean_registry_cache_for_request("sysand", package, version);

    write_root_manifest(root, package, version);
    let manifest = load_manifest(&root.join("sysml.toml")).unwrap();

    let graph_1 = resolve(&manifest, root).unwrap();
    assert_eq!(graph_1.len(), 1);
    let pkg = &graph_1.packages[0];
    assert_eq!(
        pkg.source,
        PackageSource::Registry {
            backend: "sysand".to_string(),
            package: package.to_string(),
            requested: version.to_string(),
            version: version.to_string(),
        }
    );

    let lock_1 = generate_lock(&graph_1);
    let lock_path = root.join(LOCK_FILENAME);
    save_lock(&lock_path, &lock_1).unwrap();
    let bytes_1 = fs::read(&lock_path).unwrap();

    let graph_2 = resolve(&manifest, root).unwrap();
    let lock_2 = generate_lock(&graph_2);
    save_lock(&lock_path, &lock_2).unwrap();
    let bytes_2 = fs::read(&lock_path).unwrap();

    assert_eq!(
        bytes_1, bytes_2,
        "registry lock output should be deterministic"
    );
    let loaded = load_lock(&lock_path).unwrap();
    assert!(is_lock_up_to_date(&graph_2, &loaded));
    assert_eq!(
        loaded.packages[0].source,
        format!("registry:sysand:{package}@{version}")
    );
    assert_eq!(loaded.packages[0].requested.as_deref(), Some(version));
    assert!(
        loaded.packages[0].checksum.is_some(),
        "registry lock entries should include checksum"
    );

    clean_registry_cache_for_request("sysand", package, version);
    let _ = fs::remove_file(archive_path);
}

#[test]
fn registry_cached_artifact_checksum_mismatch_reports_actionable_error() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let package = "units-registry-it-b";
    let version = "3.4.5";
    let archive_path = setup_registry_fixture(root, package, version);
    clean_registry_cache_for_request("sysand", package, version);

    write_root_manifest(root, package, version);
    let manifest = load_manifest(&root.join("sysml.toml")).unwrap();

    let first = resolve(&manifest, root).unwrap();
    assert_eq!(first.len(), 1);

    let expected_hex = sha256_hex_file(&archive_path);
    let cached_archive = registry_cache_dir_for_request("sysand", package, version)
        .join("artifacts")
        .join(format!("{expected_hex}.kpar"));
    assert!(cached_archive.exists());

    fs::write(&cached_archive, b"corrupt-registry-artifact").unwrap();

    let err = resolve(&manifest, root).unwrap_err();
    assert!(
        matches!(err, ResolveError::ChecksumMismatch { .. }),
        "expected checksum mismatch, got: {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("checksum mismatch") && msg.contains("remove cache"),
        "expected actionable checksum mismatch message, got: {msg}"
    );

    clean_registry_cache_for_request("sysand", package, version);
}

#[test]
fn registry_range_selects_highest_compatible_release_deterministically() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let package = "units-registry-it-range";
    let requirement = "^1.2";
    let selected_version = "1.9.4";
    let releases = [
        (
            "1.2.0",
            setup_registry_release_artifact(root, package, "1.2.0"),
        ),
        (
            "1.4.7",
            setup_registry_release_artifact(root, package, "1.4.7"),
        ),
        (
            "1.9.4",
            setup_registry_release_artifact(root, package, "1.9.4"),
        ),
        (
            "2.0.0",
            setup_registry_release_artifact(root, package, "2.0.0"),
        ),
    ];
    write_sysand_index_with_releases(root, package, &releases);
    clean_registry_cache_for_request("sysand", package, requirement);

    write_root_manifest(root, package, requirement);
    let manifest = load_manifest(&root.join("sysml.toml")).unwrap();

    let graph_1 = resolve(&manifest, root).unwrap();
    assert_eq!(graph_1.len(), 1);
    let pkg = &graph_1.packages[0];
    assert_eq!(
        pkg.source,
        PackageSource::Registry {
            backend: "sysand".to_string(),
            package: package.to_string(),
            requested: requirement.to_string(),
            version: selected_version.to_string(),
        }
    );

    let lock_1 = generate_lock(&graph_1);
    let lock_path = root.join(LOCK_FILENAME);
    save_lock(&lock_path, &lock_1).unwrap();
    let bytes_1 = fs::read(&lock_path).unwrap();

    let graph_2 = resolve(&manifest, root).unwrap();
    let lock_2 = generate_lock(&graph_2);
    save_lock(&lock_path, &lock_2).unwrap();
    let bytes_2 = fs::read(&lock_path).unwrap();
    assert_eq!(
        bytes_1, bytes_2,
        "range lock output should be deterministic"
    );

    let loaded = load_lock(&lock_path).unwrap();
    assert!(is_lock_up_to_date(&graph_2, &loaded));
    assert_eq!(
        loaded.packages[0].source,
        format!("registry:sysand:{package}@{selected_version}")
    );
    assert_eq!(loaded.packages[0].requested.as_deref(), Some(requirement));
    assert!(
        loaded.packages[0].checksum.is_some(),
        "registry lock entries should include checksum"
    );

    clean_registry_cache_for_request("sysand", package, requirement);
}

#[test]
fn registry_range_no_match_returns_actionable_error() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let package = "units-registry-it-nomatch";
    let requirement = "~1.5";
    let releases = [
        (
            "1.2.0",
            setup_registry_release_artifact(root, package, "1.2.0"),
        ),
        (
            "1.4.9",
            setup_registry_release_artifact(root, package, "1.4.9"),
        ),
    ];
    write_sysand_index_with_releases(root, package, &releases);
    clean_registry_cache_for_request("sysand", package, requirement);
    write_root_manifest(root, package, requirement);
    let manifest = load_manifest(&root.join("sysml.toml")).unwrap();

    let err = resolve(&manifest, root).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("no compatible release") && msg.contains(requirement),
        "expected actionable no-compatible-release error, got: {msg}"
    );
    assert!(
        !root.join(LOCK_FILENAME).exists(),
        "sysml.lock should not be generated when range cannot be resolved"
    );

    clean_registry_cache_for_request("sysand", package, requirement);
}

fn write_root_manifest(root: &Path, dep_name: &str, version: &str) {
    fs::write(
        root.join("sysml.toml"),
        format!(
            "[project]\nname = \"root\"\nversion = \"0.1.0\"\n\n[dependencies]\n{dep_name} = \"{version}\"\n"
        ),
    )
    .unwrap();
}

fn setup_registry_fixture(root: &Path, package: &str, version: &str) -> PathBuf {
    let index_dir = root.join(".sysml/registries/sysand");
    fs::create_dir_all(index_dir.join("artifacts")).unwrap();
    let archive_path = index_dir.join(format!("artifacts/{package}-{version}.kpar"));
    write_fixture_kpar(&archive_path, package, version);
    let checksum = format!("sha256:{}", sha256_hex_file(&archive_path));
    fs::write(
        index_dir.join("index.json"),
        format!(
            "{{\"packages\":{{\"{package}\":{{\"{version}\":{{\"artifact\":\"artifacts/{package}-{version}.kpar\",\"checksum\":\"{checksum}\"}}}}}}}}"
        ),
    )
    .unwrap();
    archive_path
}

fn setup_registry_release_artifact(root: &Path, package: &str, version: &str) -> PathBuf {
    let index_dir = root.join(".sysml/registries/sysand");
    fs::create_dir_all(index_dir.join("artifacts")).unwrap();
    let artifact = index_dir.join(format!("artifacts/{package}-{version}.kpar"));
    write_fixture_kpar(&artifact, package, version);
    artifact
}

fn write_sysand_index_with_releases(root: &Path, package: &str, releases: &[(&str, PathBuf)]) {
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

    let index_dir = root.join(".sysml/registries/sysand");
    fs::create_dir_all(&index_dir).unwrap();
    fs::write(
        index_dir.join("index.json"),
        format!("{{\"packages\":{{\"{package}\":{{{entries}}}}}}}"),
    )
    .unwrap();
}

fn write_fixture_kpar(path: &Path, name: &str, version: &str) {
    let mut metadata = ProjectMetadata::new();
    metadata.created = Some("2026-03-01T00:00:00Z".to_string());
    metadata.add_index_entry("Root", "Root.sysml");

    let archive = KparArchive {
        root_dir: name.to_string(),
        project_info: ProjectInfo {
            name: name.to_string(),
            version: version.to_string(),
            description: Some("registry fixture archive".to_string()),
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

fn cache_root() -> PathBuf {
    if let Some(project_dirs) = ProjectDirs::from("", "", "sysml-rs") {
        return project_dirs.cache_dir().to_path_buf();
    }

    if let Some(base_dirs) = BaseDirs::new() {
        return base_dirs.cache_dir().join("sysml-rs");
    }

    PathBuf::from("/tmp/sysml-rs-cache")
}

fn registry_cache_dir_for_request(backend: &str, package: &str, requirement: &str) -> PathBuf {
    let request_key = format!("{backend}:{package}@{requirement}");
    cache_root()
        .join("dependencies")
        .join("registry")
        .join(backend)
        .join(source_hash(&request_key))
}

fn clean_registry_cache_for_request(backend: &str, package: &str, requirement: &str) {
    let cache_dir = registry_cache_dir_for_request(backend, package, requirement);
    let _ = fs::remove_dir_all(cache_dir);
}

fn source_hash(source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn sha256_hex_file(path: &Path) -> String {
    let bytes = fs::read(path).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
