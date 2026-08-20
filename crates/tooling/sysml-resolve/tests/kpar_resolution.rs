use std::fs;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;

use sha2::{Digest, Sha256};
use sysml_project::kpar::{write_kpar, KparArchive, ProjectInfo, ProjectMetadata};
use sysml_manifest::{load_lock, load_manifest, save_lock, LOCK_FILENAME};
use sysml_resolve::{generate_lock, is_lock_up_to_date, resolve, PackageSource, ResolveError};
use tempfile::TempDir;

#[test]
fn kpar_local_path_and_file_url_lock_generation_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    fs::create_dir_all(&root).unwrap();

    let dep_dir = tmp.path().join("deps");
    fs::create_dir_all(&dep_dir).unwrap();
    let kpar_path = dep_dir.join("archive-lib.kpar");
    write_fixture_kpar(&kpar_path, "archive-lib", "1.0.0");

    clean_kpar_cache_for_source(&kpar_path);

    let file_url = format!("file://{}", kpar_path.canonicalize().unwrap().display());

    // First with local path source string.
    fs::write(
        root.join("sysml.toml"),
        "[project]\nname = \"root\"\nversion = \"0.1.0\"\n\n[dependencies]\narchive-lib = { kpar = \"../deps/archive-lib.kpar\" }\n",
    )
    .unwrap();
    assert_deterministic_lock(
        &root,
        PackageSource::Kpar {
            url: "../deps/archive-lib.kpar".to_string(),
        },
    );

    // Then with file:// source string.
    fs::write(
        root.join("sysml.toml"),
        format!(
            "[project]\nname = \"root\"\nversion = \"0.1.0\"\n\n[dependencies]\narchive-lib = {{ kpar = \"{file_url}\" }}\n"
        ),
    )
    .unwrap();
    assert_deterministic_lock(&root, PackageSource::Kpar { url: file_url });

    clean_kpar_cache_for_source(&kpar_path);
}

#[test]
fn kpar_checksum_mismatch_from_corrupt_cache_reports_actionable_error() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    fs::create_dir_all(&root).unwrap();

    let dep_dir = tmp.path().join("deps");
    fs::create_dir_all(&dep_dir).unwrap();
    let kpar_path = dep_dir.join("archive-lib.kpar");
    write_fixture_kpar(&kpar_path, "archive-lib", "1.0.0");

    clean_kpar_cache_for_source(&kpar_path);

    fs::write(
        root.join("sysml.toml"),
        "[project]\nname = \"root\"\nversion = \"0.1.0\"\n\n[dependencies]\narchive-lib = { kpar = \"../deps/archive-lib.kpar\" }\n",
    )
    .unwrap();

    let manifest = load_manifest(&root.join("sysml.toml")).unwrap();
    let graph = resolve(&manifest, &root).unwrap();
    assert_eq!(graph.len(), 1);

    let canonical_source = kpar_path.canonicalize().unwrap();
    let source_key = canonical_file_source(&canonical_source);
    let expected_hex = sha256_hex_file(&canonical_source);
    let cached_archive = kpar_cache_dir_for_source(&source_key)
        .join("archives")
        .join(format!("{expected_hex}.kpar"));
    assert!(cached_archive.exists());

    fs::write(&cached_archive, b"corrupt-cache").unwrap();

    let err = resolve(&manifest, &root).unwrap_err();
    assert!(matches!(&err, ResolveError::ChecksumMismatch { .. }));
    let message = err.to_string();
    assert!(message.contains("checksum mismatch"));
    assert!(
        message.contains("remove cache"),
        "expected actionable checksum mismatch message, got: {message}"
    );

    assert!(
        !root.join(LOCK_FILENAME).exists(),
        "lock file should not be written on checksum mismatch"
    );

    clean_kpar_cache_for_source(&kpar_path);
}

#[test]
fn kpar_http_url_lock_generation_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    fs::create_dir_all(&root).unwrap();

    let archive_path = tmp.path().join("archive-lib.kpar");
    write_fixture_kpar(&archive_path, "archive-lib", "1.0.0");
    let bytes = fs::read(&archive_path).unwrap();
    let (url, handle) = serve_bytes_n(bytes, 2);

    clean_kpar_cache_for_source_key(&url);
    fs::write(
        root.join("sysml.toml"),
        format!(
            "[project]\nname = \"root\"\nversion = \"0.1.0\"\n\n[dependencies]\narchive-lib = {{ kpar = \"{url}\" }}\n"
        ),
    )
    .unwrap();

    assert_deterministic_lock(&root, PackageSource::Kpar { url: url.clone() });

    handle.join().unwrap();
    clean_kpar_cache_for_source_key(&url);
}

#[test]
fn kpar_usage_dependency_resolves_transitively() {
    let tmp = TempDir::new().unwrap();
    let root_dir = tmp.path().join("root");
    fs::create_dir_all(&root_dir).unwrap();
    let deps_dir = tmp.path().join("deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let leaf_path = deps_dir.join("leaf-lib.kpar");
    write_fixture_kpar(&leaf_path, "leaf-lib", "0.2.0");
    let leaf_url = format!("file://{}", leaf_path.canonicalize().unwrap().display());

    let archive_path = deps_dir.join("archive-lib.kpar");
    write_fixture_kpar_with_usage(
        &archive_path,
        "archive-lib",
        "1.0.0",
        vec![leaf_url.clone()],
    );

    clean_kpar_cache_for_source(&leaf_path);
    clean_kpar_cache_for_source(&archive_path);

    fs::write(
        root_dir.join("sysml.toml"),
        "[project]\nname = \"root\"\nversion = \"0.1.0\"\n\n[dependencies]\narchive-lib = { kpar = \"../deps/archive-lib.kpar\" }\n",
    )
    .unwrap();

    let manifest = load_manifest(&root_dir.join("sysml.toml")).unwrap();
    let graph = resolve(&manifest, &root_dir).unwrap();

    assert_eq!(graph.len(), 2);
    assert_eq!(graph.packages[0].name, "leaf-lib");
    assert_eq!(graph.packages[1].name, "archive-lib");
    assert_eq!(
        graph.packages[0].source,
        PackageSource::Kpar {
            url: leaf_url.clone()
        }
    );

    clean_kpar_cache_for_source(&leaf_path);
    clean_kpar_cache_for_source(&archive_path);
}

fn assert_deterministic_lock(root: &Path, expected_source: PackageSource) {
    let manifest = load_manifest(&root.join("sysml.toml")).unwrap();

    let graph_1 = resolve(&manifest, root).unwrap();
    assert_eq!(graph_1.len(), 1);
    let pkg = &graph_1.packages[0];
    assert_eq!(pkg.name, "archive-lib");
    assert_eq!(pkg.source, expected_source);

    let lock_1 = generate_lock(&graph_1);
    assert_eq!(lock_1.packages.len(), 1);
    let lock_pkg = &lock_1.packages[0];
    assert!(
        lock_pkg
            .checksum
            .as_deref()
            .unwrap_or_default()
            .starts_with("sha256:"),
        "kpar lock entries should include checksum"
    );

    let lock_path = root.join(LOCK_FILENAME);
    save_lock(&lock_path, &lock_1).unwrap();
    let bytes_1 = fs::read(&lock_path).unwrap();

    let graph_2 = resolve(&manifest, root).unwrap();
    let lock_2 = generate_lock(&graph_2);
    save_lock(&lock_path, &lock_2).unwrap();
    let bytes_2 = fs::read(&lock_path).unwrap();

    assert_eq!(bytes_1, bytes_2, "kpar lock output should be deterministic");

    let loaded = load_lock(&lock_path).unwrap();
    assert!(is_lock_up_to_date(&graph_2, &loaded));
}

fn write_fixture_kpar(path: &Path, name: &str, version: &str) {
    write_fixture_kpar_with_usage(path, name, version, Vec::new());
}

fn write_fixture_kpar_with_usage(
    path: &Path,
    name: &str,
    version: &str,
    usage_resources: Vec<String>,
) {
    let mut metadata = ProjectMetadata::new();
    metadata.created = Some("2026-03-01T00:00:00Z".to_string());
    metadata.add_index_entry("ArchiveRoot", "ArchiveRoot.sysml");

    let archive = KparArchive {
        root_dir: name.to_string(),
        project_info: ProjectInfo {
            name: name.to_string(),
            version: version.to_string(),
            description: Some("integration fixture".to_string()),
            license: Some("MIT".to_string()),
            usage: usage_resources
                .into_iter()
                .map(|resource| sysml_project::kpar::UsageEntry {
                    resource,
                    version_constraint: None,
                })
                .collect(),
        },
        metadata,
        source_files: vec![(
            "ArchiveRoot.sysml".to_string(),
            b"package ArchiveRoot {\n  part def Sensor;\n}\n".to_vec(),
        )],
    };

    write_kpar(path, &archive).unwrap();
}

fn clean_kpar_cache_for_source(source_path: &Path) {
    let canonical = source_path.canonicalize().unwrap();
    let source_key = canonical_file_source(&canonical);
    clean_kpar_cache_for_source_key(&source_key);
}

fn clean_kpar_cache_for_source_key(source_key: &str) {
    let cache_dir = kpar_cache_dir_for_source(&source_key);
    let _ = fs::remove_dir_all(cache_dir);
}

fn kpar_cache_dir_for_source(source_key: &str) -> PathBuf {
    cache_root()
        .join("dependencies")
        .join("kpar")
        .join(source_hash(source_key))
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

fn cache_root() -> PathBuf {
    if let Some(project_dirs) = directories::ProjectDirs::from("", "", "sysml-rs") {
        return project_dirs.cache_dir().to_path_buf();
    }

    if let Some(base_dirs) = directories::BaseDirs::new() {
        return base_dirs.cache_dir().join("sysml-rs");
    }

    PathBuf::from("/tmp/sysml-rs-cache")
}

fn serve_bytes_n(bytes: Vec<u8>, requests: usize) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = thread::spawn(move || {
        for _ in 0..requests {
            let (mut stream, _) = listener.accept().unwrap();
            let mut req = [0u8; 1024];
            let _ = stream.read(&mut req);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            );
            stream.write_all(header.as_bytes()).unwrap();
            stream.write_all(&bytes).unwrap();
        }
    });

    (format!("http://{addr}/archive-lib.kpar"), handle)
}
