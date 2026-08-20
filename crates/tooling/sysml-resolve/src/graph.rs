//! Resolved dependency graph types.

use std::path::PathBuf;

/// A fully resolved dependency graph.
///
/// Packages are ordered in topological order: dependencies come before
/// the packages that depend on them (post-order DFS). This means the
/// root project's own package entry, if any, comes last.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedGraph {
    /// Resolved packages in dependency order (deps before dependents).
    pub packages: Vec<ResolvedPackage>,
}

impl ResolvedGraph {
    /// Create an empty resolved graph.
    pub fn new() -> Self {
        ResolvedGraph {
            packages: Vec::new(),
        }
    }

    /// Find a resolved package by name.
    pub fn find(&self, name: &str) -> Option<&ResolvedPackage> {
        self.packages.iter().find(|p| p.name == name)
    }

    /// Returns true if the graph contains a package with the given name.
    pub fn contains(&self, name: &str) -> bool {
        self.packages.iter().any(|p| p.name == name)
    }

    /// Returns the number of packages in the graph.
    pub fn len(&self) -> usize {
        self.packages.len()
    }

    /// Returns true if the graph is empty.
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }
}

impl Default for ResolvedGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// A single resolved package in the dependency graph.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPackage {
    /// Package name (from `[project].name` in the dependency's `sysml.toml`).
    pub name: String,

    /// Package version (from `[project].version`).
    pub version: String,

    /// Source type of this dependency.
    pub source: PackageSource,

    /// Directory containing the package's `.sysml` source files.
    ///
    /// For path deps, this is the resolved absolute path to the dep dir.
    /// For git/kpar/registry deps, this is the per-source cache dir into
    /// which the provider materialized the package's sources.
    pub source_dir: PathBuf,
}

/// The origin of a resolved package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageSource {
    /// A local path dependency (`path = "../../foo"`).
    /// Contains the original (possibly relative) path as specified.
    Path(String),

    /// A git repository dependency.
    /// Contains the URL and resolved commit hash.
    Git { url: String, commit: String },

    /// A KPAR archive dependency.
    /// Contains the download URL.
    Kpar { url: String },

    /// The SysML standard library (built-in).
    Stdlib,

    /// A registry/version-constraint dependency.
    /// Contains backend id, package name, requested requirement, and resolved
    /// version string.
    Registry {
        backend: String,
        package: String,
        requested: String,
        version: String,
    },
}

impl PackageSource {
    /// Returns a compact string descriptor suitable for lock file storage.
    pub fn to_lock_source(&self) -> String {
        match self {
            PackageSource::Path(p) => format!("path:{p}"),
            PackageSource::Git { url, commit } => format!("git:{url}#{commit}"),
            PackageSource::Kpar { url } => format!("kpar:{url}"),
            PackageSource::Stdlib => "stdlib".to_owned(),
            PackageSource::Registry {
                backend,
                package,
                requested: _,
                version,
            } => format!("registry:{backend}:{package}@{version}"),
        }
    }
}
