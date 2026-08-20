//! Dependency resolver implementation.
//!
//! Resolves a [`SysmlManifest`] into a [`ResolvedGraph`] by:
//! 1. Delegating source-specific handling to source providers
//! 2. Detecting and reporting dependency cycles
//! 3. Emitting packages in topological order (dependencies first)

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use sysml_manifest::{Dependency, SysmlManifest};
use tracing::{debug, trace, warn};

use crate::error::ResolveError;
use crate::graph::{ResolvedGraph, ResolvedPackage};
use crate::providers::{ProviderResolution, SourceProviderRegistry};

/// Resolve the full dependency graph for a project.
///
/// `manifest` is the root project's parsed `sysml.toml`.
/// `manifest_dir` is the directory that contains the root `sysml.toml`
/// (used as the base for resolving relative path dependencies).
///
/// Returns a [`ResolvedGraph`] with packages in topological order
/// (dependencies before dependents). The root package itself is NOT
/// included in the graph — only its transitive dependencies.
///
/// # Errors
///
/// - [`ResolveError::Cycle`] if a dependency cycle is detected
/// - [`ResolveError::MissingDependency`] if a path dep directory is missing
/// - [`ResolveError::Manifest`] if a dependency's `sysml.toml` is invalid
/// - [`ResolveError::UnsupportedSource`] for source types not implemented yet
pub fn resolve(
    manifest: &SysmlManifest,
    manifest_dir: &Path,
) -> Result<ResolvedGraph, ResolveError> {
    let started = Instant::now();
    let result = ResolverEngine::new().resolve(manifest, manifest_dir);
    let elapsed_ms = started.elapsed().as_millis() as u64;

    match &result {
        Ok(graph) => {
            debug!(
                project = %manifest.project.name,
                declared_dependencies = manifest.dependencies.len(),
                resolved_packages = graph.packages.len(),
                elapsed_ms,
                "dependency resolution complete"
            );
        }
        Err(error) => {
            warn!(
                project = %manifest.project.name,
                declared_dependencies = manifest.dependencies.len(),
                elapsed_ms,
                error = %error,
                "dependency resolution failed"
            );
        }
    }

    result
}

/// Collect all `.sysml` source files from all packages in a resolved graph.
///
/// For each package, recursively walks its `source_dir` and collects every
/// file with a `.sysml` extension. Files are returned in a stable order
/// (dependency packages first, then dependents — matching the graph order).
pub fn source_paths(graph: &ResolvedGraph) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for pkg in &graph.packages {
        collect_sysml_files(&pkg.source_dir, &mut paths);
    }
    paths
}

// ---------------------------------------------------------------------------
// Resolver engine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PackageIdentity {
    name: String,
    source_fingerprint: String,
}

impl PackageIdentity {
    fn from_resolution(resolved: &ProviderResolution) -> Self {
        PackageIdentity {
            name: resolved.manifest.project.name.clone(),
            source_fingerprint: resolved.source_fingerprint(),
        }
    }
}

struct ResolverEngine {
    graph: ResolvedGraph,
    providers: SourceProviderRegistry,
    // Dedup by package identity key (name + source fingerprint).
    visited: HashSet<PackageIdentity>,
    // Track current DFS package-name stack for cycle diagnostics.
    path_stack: Vec<String>,
    // Track directories in current DFS stack for cycle detection.
    in_stack: HashSet<PathBuf>,
}

impl ResolverEngine {
    fn new() -> Self {
        ResolverEngine {
            graph: ResolvedGraph::new(),
            providers: SourceProviderRegistry::new(),
            visited: HashSet::new(),
            path_stack: Vec::new(),
            in_stack: HashSet::new(),
        }
    }

    fn resolve(
        mut self,
        manifest: &SysmlManifest,
        manifest_dir: &Path,
    ) -> Result<ResolvedGraph, ResolveError> {
        self.resolve_deps(manifest, manifest_dir)?;
        Ok(self.graph)
    }

    fn resolve_deps(
        &mut self,
        manifest: &SysmlManifest,
        manifest_dir: &Path,
    ) -> Result<(), ResolveError> {
        trace!(
            project = %manifest.project.name,
            manifest_dir = %manifest_dir.display(),
            dependency_count = manifest.dependencies.len(),
            "resolving manifest dependency set"
        );

        let canonical_dir = manifest_dir
            .canonicalize()
            .map_err(|e| ResolveError::io(manifest_dir, e))?;

        self.path_stack.push(manifest.project.name.clone());
        self.in_stack.insert(canonical_dir.clone());

        let result = (|| {
            for (dep_name, dep_spec) in &manifest.dependencies {
                trace!(
                    requested_name = %dep_name,
                    source = dependency_source_kind(dep_spec),
                    "resolving dependency entry"
                );
                let resolved = self.providers.resolve(dep_name, dep_spec, manifest_dir)?;
                self.resolve_dependency(dep_name, resolved)?;
            }

            Ok(())
        })();

        self.path_stack.pop();
        self.in_stack.remove(&canonical_dir);

        result
    }

    fn resolve_dependency(
        &mut self,
        requested_name: &str,
        resolved: ProviderResolution,
    ) -> Result<(), ResolveError> {
        // Check for cycle: is this source directory already in current DFS path?
        if self.in_stack.contains(&resolved.source_dir) {
            let mut cycle = self.path_stack.clone();
            cycle.push(requested_name.to_owned());
            warn!(
                requested_name = %requested_name,
                cycle = ?cycle,
                "dependency cycle detected"
            );
            return Err(ResolveError::Cycle { cycle });
        }

        // Already fully resolved — skip (handles diamond deps).
        let identity = PackageIdentity::from_resolution(&resolved);
        if self.visited.contains(&identity) {
            trace!(
                requested_name = %requested_name,
                package = %resolved.manifest.project.name,
                source = package_source_kind(&resolved.source),
                "dependency already resolved; skipping duplicate"
            );
            return Ok(());
        }

        let dep_name = resolved.manifest.project.name.clone();
        let dep_version = resolved.manifest.project.version.clone();
        let dep_manifest = resolved.manifest;
        let dep_dir = resolved.source_dir;
        let dep_source = resolved.source;

        debug!(
            requested_name = %requested_name,
            package = %dep_name,
            source = package_source_kind(&dep_source),
            manifest_dir = %dep_dir.display(),
            declared_dependencies = dep_manifest.dependencies.len(),
            "resolving dependency package"
        );

        // Recursively resolve dependency's own dependencies first (post-order DFS).
        self.resolve_deps(&dep_manifest, &dep_dir)?;

        self.visited.insert(identity);
        self.graph.packages.push(ResolvedPackage {
            name: dep_name,
            version: dep_version,
            source: dep_source,
            source_dir: dep_dir,
        });

        trace!(
            package_count = self.graph.packages.len(),
            "dependency package appended to resolved graph"
        );

        Ok(())
    }
}

fn dependency_source_kind(dep: &Dependency) -> &'static str {
    if dep.is_path() {
        "path"
    } else if dep.is_git() {
        "git"
    } else if dep.is_kpar() {
        "kpar"
    } else if dep.is_registry() {
        "registry"
    } else {
        "unknown"
    }
}

fn package_source_kind(source: &crate::graph::PackageSource) -> &'static str {
    match source {
        crate::graph::PackageSource::Path(_) => "path",
        crate::graph::PackageSource::Git { .. } => "git",
        crate::graph::PackageSource::Kpar { .. } => "kpar",
        crate::graph::PackageSource::Registry { .. } => "registry",
        crate::graph::PackageSource::Stdlib => "stdlib",
    }
}

/// Recursively collect all `.sysml` files under a directory.
fn collect_sysml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    let mut subdirs = Vec::new();
    let mut files = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            subdirs.push(path);
        } else if path.extension().and_then(|e| e.to_str()) == Some("sysml") {
            files.push(path);
        }
    }

    // Sort for deterministic ordering.
    files.sort();
    subdirs.sort();

    out.extend(files);
    for subdir in subdirs {
        collect_sysml_files(&subdir, out);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::graph::PackageSource;
    use std::fs;
    use sysml_project::kpar::{write_kpar, KparArchive, ProjectInfo, ProjectMetadata};
    use tempfile::TempDir;

    fn write_manifest(dir: &Path, name: &str, version: &str, deps: &[(&str, &str)]) {
        let mut content = format!("[project]\nname = \"{name}\"\nversion = \"{version}\"\n");
        if !deps.is_empty() {
            content.push_str("\n[dependencies]\n");
            for (dep_name, dep_path) in deps {
                content.push_str(&format!("{dep_name} = {{ path = \"{dep_path}\" }}\n"));
            }
        }
        fs::write(dir.join("sysml.toml"), content).unwrap();
    }

    fn write_sysml(dir: &Path, name: &str) {
        fs::write(dir.join(name), format!("// {name}\n")).unwrap();
    }

    #[test]
    fn resolve_no_deps() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_manifest(root, "my-project", "0.1.0", &[]);

        let manifest = sysml_manifest::load_manifest(&root.join("sysml.toml")).unwrap();
        let graph = resolve(&manifest, root).unwrap();
        assert!(graph.is_empty());
    }

    #[test]
    fn resolve_single_path_dep() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let lib = root.join("lib");
        fs::create_dir_all(&lib).unwrap();

        write_manifest(&lib, "my-lib", "0.2.0", &[]);
        write_sysml(&lib, "types.sysml");
        write_manifest(root, "my-project", "0.1.0", &[("my-lib", "./lib")]);

        let manifest = sysml_manifest::load_manifest(&root.join("sysml.toml")).unwrap();
        let graph = resolve(&manifest, root).unwrap();

        assert_eq!(graph.len(), 1);
        let pkg = graph.find("my-lib").unwrap();
        assert_eq!(pkg.name, "my-lib");
        assert_eq!(pkg.version, "0.2.0");
        assert_eq!(pkg.source, PackageSource::Path("./lib".to_string()));
        assert_eq!(pkg.source_dir, lib.canonicalize().unwrap());
    }

    #[test]
    fn resolve_transitive_deps() {
        // root -> middle -> leaf
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let middle = root.join("middle");
        let leaf = root.join("leaf");
        fs::create_dir_all(&middle).unwrap();
        fs::create_dir_all(&leaf).unwrap();

        write_manifest(&leaf, "leaf-lib", "0.1.0", &[]);
        write_manifest(&middle, "middle-lib", "0.2.0", &[("leaf-lib", "../leaf")]);
        write_manifest(root, "root-project", "0.1.0", &[("middle-lib", "./middle")]);

        let manifest = sysml_manifest::load_manifest(&root.join("sysml.toml")).unwrap();
        let graph = resolve(&manifest, root).unwrap();

        assert_eq!(graph.len(), 2);
        // leaf comes before middle (topological order).
        assert_eq!(graph.packages[0].name, "leaf-lib");
        assert_eq!(graph.packages[1].name, "middle-lib");
    }

    #[test]
    fn resolve_diamond_dep() {
        // root -> left -> base
        //       -> right -> base
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let left = root.join("left");
        let right = root.join("right");
        let base = root.join("base");
        fs::create_dir_all(&left).unwrap();
        fs::create_dir_all(&right).unwrap();
        fs::create_dir_all(&base).unwrap();

        write_manifest(&base, "base-lib", "1.0.0", &[]);
        write_manifest(&left, "left-lib", "0.1.0", &[("base-lib", "../base")]);
        write_manifest(&right, "right-lib", "0.1.0", &[("base-lib", "../base")]);
        write_manifest(
            root,
            "root-project",
            "0.1.0",
            &[("left-lib", "./left"), ("right-lib", "./right")],
        );

        let manifest = sysml_manifest::load_manifest(&root.join("sysml.toml")).unwrap();
        let graph = resolve(&manifest, root).unwrap();

        // base appears exactly once, left and right each once.
        assert_eq!(graph.len(), 3);
        let names: Vec<&str> = graph.packages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names[0], "base-lib"); // base first
                                          // left and right can be in either order
        assert!(names.contains(&"left-lib"));
        assert!(names.contains(&"right-lib"));
    }

    #[test]
    fn resolve_distinct_source_fingerprints_for_same_name_are_kept() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let shared = root.join("shared");
        fs::create_dir_all(&shared).unwrap();

        write_manifest(&shared, "shared-lib", "1.0.0", &[]);
        write_manifest(
            root,
            "root-project",
            "0.1.0",
            &[("shared-a", "./shared"), ("shared-b", "././shared")],
        );

        let manifest = sysml_manifest::load_manifest(&root.join("sysml.toml")).unwrap();
        let graph = resolve(&manifest, root).unwrap();

        assert_eq!(graph.len(), 2);
        let mut sources: Vec<String> = graph
            .packages
            .iter()
            .map(|p| p.source.to_lock_source())
            .collect();
        sources.sort();
        assert_eq!(sources, vec!["path:././shared", "path:./shared"]);
    }

    #[test]
    fn resolve_detects_direct_cycle() {
        // a -> b -> a
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let dep_a = root.join("dep-a");
        let dep_b = root.join("dep-b");
        fs::create_dir_all(&dep_a).unwrap();
        fs::create_dir_all(&dep_b).unwrap();

        write_manifest(&dep_a, "dep-a", "0.1.0", &[("dep-b", "../dep-b")]);
        write_manifest(&dep_b, "dep-b", "0.1.0", &[("dep-a", "../dep-a")]);
        write_manifest(root, "root-project", "0.1.0", &[("dep-a", "./dep-a")]);

        let manifest = sysml_manifest::load_manifest(&root.join("sysml.toml")).unwrap();
        let result = resolve(&manifest, root);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ResolveError::Cycle { .. }));
        if let ResolveError::Cycle { cycle } = err {
            assert!(cycle.contains(&"dep-a".to_string()) || cycle.contains(&"dep-b".to_string()));
        }
    }

    #[test]
    fn resolve_missing_dep_dir() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_manifest(
            root,
            "my-project",
            "0.1.0",
            &[("missing-lib", "./does-not-exist")],
        );

        let manifest = sysml_manifest::load_manifest(&root.join("sysml.toml")).unwrap();
        let result = resolve(&manifest, root);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ResolveError::MissingDependency { .. }
        ));
    }

    #[test]
    fn resolve_missing_manifest_in_dep() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let lib = root.join("lib");
        fs::create_dir_all(&lib).unwrap();
        // lib directory exists but has no sysml.toml
        write_manifest(root, "my-project", "0.1.0", &[("my-lib", "./lib")]);

        let manifest = sysml_manifest::load_manifest(&root.join("sysml.toml")).unwrap();
        let result = resolve(&manifest, root);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ResolveError::MissingDependency { .. }
        ));
    }

    #[test]
    fn resolve_git_dep_missing_repo_returns_io() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let content = r#"
[project]
name = "my-project"
version = "0.1.0"

[dependencies]
remote-lib = { git = "file:///definitely/missing/repo", rev = "0123456789abcdef0123456789abcdef01234567" }
"#;
        fs::write(root.join("sysml.toml"), content).unwrap();

        let manifest = sysml_manifest::load_manifest(&root.join("sysml.toml")).unwrap();
        let result = resolve(&manifest, root);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ResolveError::Io { .. }));
    }

    #[test]
    fn resolve_kpar_dep_from_local_path() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        create_kpar_archive(root, "archive-lib", "2.0.0");
        let content = r#"
[project]
name = "my-project"
version = "0.1.0"

[dependencies]
archive-lib = { kpar = "./archive-lib.kpar" }
"#;
        fs::write(root.join("sysml.toml"), content).unwrap();

        let manifest = sysml_manifest::load_manifest(&root.join("sysml.toml")).unwrap();
        let graph = resolve(&manifest, root).unwrap();

        assert_eq!(graph.len(), 1);
        let pkg = &graph.packages[0];
        assert_eq!(pkg.name, "archive-lib");
        assert_eq!(pkg.version, "2.0.0");
        assert_eq!(
            pkg.source,
            PackageSource::Kpar {
                url: "./archive-lib.kpar".to_string(),
            }
        );
        assert!(pkg.source_dir.join("sysml.toml").exists());
        assert!(pkg.source_dir.join("Root.sysml").exists());
    }

    #[test]
    fn source_paths_collects_sysml_files() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let lib = root.join("lib");
        fs::create_dir_all(&lib).unwrap();
        fs::create_dir_all(lib.join("sub")).unwrap();

        write_sysml(&lib, "types.sysml");
        write_sysml(&lib, "interfaces.sysml");
        fs::write(lib.join("readme.txt"), "not sysml").unwrap();
        write_sysml(&lib.join("sub"), "nested.sysml");

        write_manifest(&lib, "my-lib", "0.1.0", &[]);
        write_manifest(root, "root", "0.1.0", &[("my-lib", "./lib")]);

        let manifest = sysml_manifest::load_manifest(&root.join("sysml.toml")).unwrap();
        let graph = resolve(&manifest, root).unwrap();
        let paths = source_paths(&graph);

        assert_eq!(paths.len(), 3);
        let names: Vec<&str> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert!(names.contains(&"types.sysml"));
        assert!(names.contains(&"interfaces.sysml"));
        assert!(names.contains(&"nested.sysml"));
    }

    #[test]
    fn source_paths_empty_graph() {
        let graph = ResolvedGraph::new();
        let paths = source_paths(&graph);
        assert!(paths.is_empty());
    }

    fn create_kpar_archive(root: &Path, name: &str, version: &str) {
        let mut metadata = ProjectMetadata::new();
        metadata.created = Some("2026-03-01T00:00:00Z".to_string());
        metadata.add_index_entry("Root", "Root.sysml");
        let archive = KparArchive {
            root_dir: name.to_string(),
            project_info: ProjectInfo {
                name: name.to_string(),
                version: version.to_string(),
                description: Some("resolver test archive".to_string()),
                license: Some("MIT".to_string()),
                usage: Vec::new(),
            },
            metadata,
            source_files: vec![(
                "Root.sysml".to_string(),
                b"package Root {\n  part def X;\n}\n".to_vec(),
            )],
        };
        write_kpar(&root.join(format!("{name}.kpar")), &archive).unwrap();
    }
}
