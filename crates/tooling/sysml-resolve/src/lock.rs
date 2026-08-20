//! Lock file generation and change detection for resolved graphs.

use std::fs;

use sysml_manifest::{LockFile, LockedPackage};

use crate::graph::{PackageSource, ResolvedGraph};
use crate::providers::KPAR_CHECKSUM_FILENAME;

/// Generate a [`LockFile`] from a resolved dependency graph.
///
/// The generated lock file contains one entry per resolved package,
/// sorted by `(name, source)` for deterministic output. Packages
/// without checksums (e.g. path dependencies) will have `checksum = None`.
pub fn generate_lock(graph: &ResolvedGraph) -> LockFile {
    let mut lock = LockFile::new();

    for pkg in &graph.packages {
        let locked = make_locked_package(pkg);
        lock.add_package(locked);
    }

    sort_packages_for_determinism(&mut lock);
    lock
}

/// Check whether the resolved graph matches an existing lock file.
///
/// Returns `true` if the lock file is up-to-date with the resolved graph
/// (same packages, same versions, same sources). Returns `false` if the
/// lock file needs to be regenerated.
pub fn is_lock_up_to_date(graph: &ResolvedGraph, lock: &LockFile) -> bool {
    let expected = generate_lock(graph);
    let mut existing = lock.clone();
    sort_packages_for_determinism(&mut existing);

    expected.lock_version == existing.lock_version && expected.packages == existing.packages
}

fn make_locked_package(pkg: &crate::graph::ResolvedPackage) -> LockedPackage {
    match &pkg.source {
        PackageSource::Path(rel_path) => {
            LockedPackage::from_path(&pkg.name, &pkg.version, rel_path)
        }
        PackageSource::Git { url, commit } => {
            LockedPackage::from_git(&pkg.name, &pkg.version, url, commit, None)
        }
        PackageSource::Kpar { url } => {
            let checksum = kpar_checksum_from_source_dir(&pkg.source_dir);
            if let Some(checksum) = checksum {
                LockedPackage::from_kpar(&pkg.name, &pkg.version, url, checksum)
            } else {
                LockedPackage {
                    name: pkg.name.clone(),
                    version: pkg.version.clone(),
                    source: format!("kpar:{url}"),
                    checksum: None,
                    requested: None,
                }
            }
        }
        PackageSource::Stdlib => LockedPackage {
            name: pkg.name.clone(),
            version: pkg.version.clone(),
            source: "stdlib".to_owned(),
            checksum: None,
            requested: None,
        },
        PackageSource::Registry {
            backend,
            package,
            requested,
            version,
        } => LockedPackage {
            name: pkg.name.clone(),
            version: pkg.version.clone(),
            source: format!("registry:{backend}:{package}@{version}"),
            checksum: kpar_checksum_from_source_dir(&pkg.source_dir),
            requested: Some(requested.clone()),
        },
    }
}

fn kpar_checksum_from_source_dir(source_dir: &std::path::Path) -> Option<String> {
    let checksum_path = source_dir.join(KPAR_CHECKSUM_FILENAME);
    fs::read_to_string(checksum_path)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn sort_packages_for_determinism(lock: &mut LockFile) {
    lock.packages
        .sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.source.cmp(&b.source)));
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::graph::{PackageSource, ResolvedGraph, ResolvedPackage};
    use std::path::PathBuf;

    fn make_path_pkg(name: &str, version: &str, path: &str, dir: &str) -> ResolvedPackage {
        ResolvedPackage {
            name: name.to_string(),
            version: version.to_string(),
            source: PackageSource::Path(path.to_string()),
            source_dir: PathBuf::from(dir),
        }
    }

    fn make_registry_pkg(
        name: &str,
        version: &str,
        backend: &str,
        package: &str,
        requested: &str,
        dir: &std::path::Path,
    ) -> ResolvedPackage {
        ResolvedPackage {
            name: name.to_string(),
            version: version.to_string(),
            source: PackageSource::Registry {
                backend: backend.to_string(),
                package: package.to_string(),
                requested: requested.to_string(),
                version: version.to_string(),
            },
            source_dir: dir.to_path_buf(),
        }
    }

    #[test]
    fn generate_lock_empty_graph() {
        let graph = ResolvedGraph::new();
        let lock = generate_lock(&graph);
        assert_eq!(lock.lock_version, 1);
        assert!(lock.packages.is_empty());
    }

    #[test]
    fn generate_lock_with_path_deps() {
        let mut graph = ResolvedGraph::new();
        graph
            .packages
            .push(make_path_pkg("lib-a", "0.1.0", "../lib-a", "/tmp/lib-a"));
        graph
            .packages
            .push(make_path_pkg("lib-b", "0.2.0", "../lib-b", "/tmp/lib-b"));

        let lock = generate_lock(&graph);

        assert_eq!(lock.packages.len(), 2);

        // Lock is sorted alphabetically.
        let a = lock.find_package("lib-a").unwrap();
        assert_eq!(a.version, "0.1.0");
        assert_eq!(a.source, "path:../lib-a");
        assert!(a.checksum.is_none());

        let b = lock.find_package("lib-b").unwrap();
        assert_eq!(b.version, "0.2.0");
        assert_eq!(b.source, "path:../lib-b");
    }

    #[test]
    fn generate_lock_sorted() {
        let mut graph = ResolvedGraph::new();
        graph
            .packages
            .push(make_path_pkg("zebra", "1.0.0", "../z", "/tmp/z"));
        graph
            .packages
            .push(make_path_pkg("alpha", "1.0.0", "../a", "/tmp/a"));
        graph
            .packages
            .push(make_path_pkg("middle", "1.0.0", "../m", "/tmp/m"));

        let lock = generate_lock(&graph);
        let names: Vec<&str> = lock.packages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "middle", "zebra"]);
    }

    #[test]
    fn generate_lock_sorted_by_name_then_source() {
        let mut graph = ResolvedGraph::new();
        graph
            .packages
            .push(make_path_pkg("same", "1.0.0", "../z", "/tmp/z"));
        graph
            .packages
            .push(make_path_pkg("same", "1.0.0", "../a", "/tmp/a"));

        let lock = generate_lock(&graph);
        let sources: Vec<&str> = lock.packages.iter().map(|p| p.source.as_str()).collect();
        assert_eq!(sources, vec!["path:../a", "path:../z"]);
    }

    #[test]
    fn is_lock_up_to_date_matches() {
        let mut graph = ResolvedGraph::new();
        graph
            .packages
            .push(make_path_pkg("my-lib", "0.1.0", "../my-lib", "/tmp/my-lib"));

        let lock = generate_lock(&graph);
        assert!(is_lock_up_to_date(&graph, &lock));
    }

    #[test]
    fn is_lock_up_to_date_version_changed() {
        let mut graph = ResolvedGraph::new();
        graph
            .packages
            .push(make_path_pkg("my-lib", "0.2.0", "../my-lib", "/tmp/my-lib"));

        let mut lock = LockFile::new();
        lock.add_package(LockedPackage::from_path("my-lib", "0.1.0", "../my-lib"));

        assert!(!is_lock_up_to_date(&graph, &lock));
    }

    #[test]
    fn is_lock_up_to_date_missing_package() {
        let mut graph = ResolvedGraph::new();
        graph
            .packages
            .push(make_path_pkg("my-lib", "0.1.0", "../my-lib", "/tmp/my-lib"));
        graph.packages.push(make_path_pkg(
            "other-lib",
            "0.1.0",
            "../other-lib",
            "/tmp/other-lib",
        ));

        let mut lock = LockFile::new();
        lock.add_package(LockedPackage::from_path("my-lib", "0.1.0", "../my-lib"));
        // other-lib missing from lock

        assert!(!is_lock_up_to_date(&graph, &lock));
    }

    #[test]
    fn is_lock_up_to_date_source_changed() {
        let mut graph = ResolvedGraph::new();
        graph.packages.push(make_path_pkg(
            "my-lib",
            "0.1.0",
            "../new-path",
            "/tmp/new-path",
        ));

        let mut lock = LockFile::new();
        lock.add_package(LockedPackage::from_path("my-lib", "0.1.0", "../old-path"));

        assert!(!is_lock_up_to_date(&graph, &lock));
    }

    #[test]
    fn is_lock_up_to_date_handles_duplicate_names_with_different_sources() {
        let mut graph = ResolvedGraph::new();
        graph
            .packages
            .push(make_path_pkg("same", "0.1.0", "../a", "/tmp/a"));
        graph
            .packages
            .push(make_path_pkg("same", "0.1.0", "../b", "/tmp/b"));

        let lock = generate_lock(&graph);
        assert!(is_lock_up_to_date(&graph, &lock));

        let mut changed_lock = lock.clone();
        changed_lock.packages[0].source = "path:../x".to_string();
        assert!(!is_lock_up_to_date(&graph, &changed_lock));
    }

    #[test]
    fn is_lock_up_to_date_empty_both() {
        let graph = ResolvedGraph::new();
        let lock = LockFile::new();
        assert!(is_lock_up_to_date(&graph, &lock));
    }

    #[test]
    fn generate_lock_reads_kpar_checksum_marker() {
        let temp = tempfile::TempDir::new().unwrap();
        let source_dir = temp.path().join("kpar-extracted");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(
            source_dir.join(crate::providers::KPAR_CHECKSUM_FILENAME),
            "sha256:abcd1234\n",
        )
        .unwrap();

        let mut graph = ResolvedGraph::new();
        graph.packages.push(ResolvedPackage {
            name: "archive-lib".to_string(),
            version: "1.0.0".to_string(),
            source: PackageSource::Kpar {
                url: "../deps/archive-lib.kpar".to_string(),
            },
            source_dir,
        });

        let lock = generate_lock(&graph);
        assert_eq!(lock.packages.len(), 1);
        let pkg = &lock.packages[0];
        assert_eq!(pkg.source, "kpar:../deps/archive-lib.kpar");
        assert_eq!(pkg.checksum.as_deref(), Some("sha256:abcd1234"));
    }

    #[test]
    fn generate_lock_reads_registry_checksum_marker() {
        let temp = tempfile::TempDir::new().unwrap();
        let source_dir = temp.path().join("registry-extracted");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(
            source_dir.join(crate::providers::KPAR_CHECKSUM_FILENAME),
            "sha256:feedface\n",
        )
        .unwrap();

        let mut graph = ResolvedGraph::new();
        graph.packages.push(make_registry_pkg(
            "units",
            "1.4.2",
            "sysand",
            "units",
            "^1.4",
            &source_dir,
        ));

        let lock = generate_lock(&graph);
        assert_eq!(lock.packages.len(), 1);
        let pkg = &lock.packages[0];
        assert_eq!(pkg.source, "registry:sysand:units@1.4.2");
        assert_eq!(pkg.checksum.as_deref(), Some("sha256:feedface"));
        assert_eq!(pkg.requested.as_deref(), Some("^1.4"));
    }
}
