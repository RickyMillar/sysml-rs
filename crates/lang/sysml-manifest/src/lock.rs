//! `sysml.lock` lock file parsing and serialization.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::ManifestError;

/// Current lock file format version.
const LOCK_VERSION: u32 = 1;

/// A parsed `sysml.lock` lock file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockFile {
    /// Lock file format version.
    pub lock_version: u32,

    /// Resolved packages.
    #[serde(default, rename = "package")]
    pub packages: Vec<LockedPackage>,
}

/// A single resolved package entry in the lock file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedPackage {
    /// Package name.
    pub name: String,

    /// Resolved version.
    pub version: String,

    /// Source descriptor.
    ///
    /// Format examples:
    /// - `path:../local-dep`
    /// - `git:https://github.com/org/repo#commitsha`
    /// - `kpar:https://example.com/lib.kpar`
    /// - `registry:1.0.0`
    pub source: String,

    /// Content checksum (for remote sources).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,

    /// Declared source requirement when resolution pins a concrete source
    /// descriptor (for example registry ranges).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested: Option<String>,
}

impl LockFile {
    /// Create a new empty lock file.
    pub fn new() -> Self {
        LockFile {
            lock_version: LOCK_VERSION,
            packages: Vec::new(),
        }
    }

    /// Add a locked package entry.
    pub fn add_package(&mut self, package: LockedPackage) {
        self.packages.push(package);
    }

    /// Find a locked package by name.
    pub fn find_package(&self, name: &str) -> Option<&LockedPackage> {
        self.packages.iter().find(|p| p.name == name)
    }

    /// Sort packages by name for deterministic output.
    pub fn sort(&mut self) {
        self.packages.sort_by(|a, b| a.name.cmp(&b.name));
    }
}

impl Default for LockFile {
    fn default() -> Self {
        Self::new()
    }
}

impl LockedPackage {
    /// Create a locked package for a path dependency.
    pub fn from_path(name: impl Into<String>, version: impl Into<String>, path: &str) -> Self {
        LockedPackage {
            name: name.into(),
            version: version.into(),
            source: format!("path:{path}"),
            checksum: None,
            requested: None,
        }
    }

    /// Create a locked package for a git dependency.
    pub fn from_git(
        name: impl Into<String>,
        version: impl Into<String>,
        url: &str,
        commit: &str,
        checksum: Option<String>,
    ) -> Self {
        LockedPackage {
            name: name.into(),
            version: version.into(),
            source: format!("git:{url}#{commit}"),
            checksum,
            requested: None,
        }
    }

    /// Create a locked package for a KPAR URL dependency.
    pub fn from_kpar(
        name: impl Into<String>,
        version: impl Into<String>,
        url: &str,
        checksum: String,
    ) -> Self {
        LockedPackage {
            name: name.into(),
            version: version.into(),
            source: format!("kpar:{url}"),
            checksum: Some(checksum),
            requested: None,
        }
    }

    /// Returns true if this is a path source.
    pub fn is_path(&self) -> bool {
        self.source.starts_with("path:")
    }

    /// Returns true if this is a git source.
    pub fn is_git(&self) -> bool {
        self.source.starts_with("git:")
    }

    /// Returns true if this is a KPAR source.
    pub fn is_kpar(&self) -> bool {
        self.source.starts_with("kpar:")
    }
}

/// Load and parse a `sysml.lock` file from the given path.
pub fn load_lock(path: &Path) -> Result<LockFile, ManifestError> {
    let content = std::fs::read_to_string(path).map_err(|e| ManifestError::io(path, e))?;
    parse_lock(&content, path)
}

/// Parse a `sysml.lock` file from a string.
pub fn parse_lock(content: &str, path: &Path) -> Result<LockFile, ManifestError> {
    toml::from_str(content).map_err(|e| ManifestError::parse(path, e.message()))
}

/// Save a lock file to disk.
pub fn save_lock(path: &Path, lock: &LockFile) -> Result<(), ManifestError> {
    let content =
        toml::to_string_pretty(lock).map_err(|e| ManifestError::Serialize(e.to_string()))?;
    std::fs::write(path, content).map_err(|e| ManifestError::io(path, e))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parse_lock_file() {
        let toml = r#"
lock_version = 1

[[package]]
name = "beverage-types"
version = "0.1.0"
source = "path:../beverage-types"

[[package]]
name = "thermal-model"
version = "1.0.0"
source = "git:https://github.com/acme/thermal-model#abc123def"
checksum = "sha256:4da8b89a"
"#;
        let lock = parse_lock(toml, Path::new("test.lock")).unwrap();
        assert_eq!(lock.lock_version, 1);
        assert_eq!(lock.packages.len(), 2);

        let bev = lock.find_package("beverage-types").unwrap();
        assert_eq!(bev.version, "0.1.0");
        assert!(bev.is_path());
        assert!(bev.checksum.is_none());

        let thermal = lock.find_package("thermal-model").unwrap();
        assert_eq!(thermal.version, "1.0.0");
        assert!(thermal.is_git());
        assert_eq!(thermal.checksum.as_deref(), Some("sha256:4da8b89a"));
    }

    #[test]
    fn round_trip_lock_file() {
        let mut original = LockFile::new();
        original.add_package(LockedPackage::from_path(
            "local-dep",
            "0.1.0",
            "../local-dep",
        ));
        original.add_package(LockedPackage::from_git(
            "remote-dep",
            "1.0.0",
            "https://github.com/org/repo",
            "abc123",
            Some("sha256:deadbeef".to_string()),
        ));
        original.add_package(LockedPackage::from_kpar(
            "archive-dep",
            "2.0.0",
            "https://example.com/lib.kpar",
            "sha256:cafebabe".to_string(),
        ));

        let serialized = toml::to_string_pretty(&original).unwrap();
        let deserialized: LockFile = toml::from_str(&serialized).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn lock_file_sort() {
        let mut lock = LockFile::new();
        lock.add_package(LockedPackage::from_path("zebra", "1.0.0", "../z"));
        lock.add_package(LockedPackage::from_path("alpha", "1.0.0", "../a"));
        lock.add_package(LockedPackage::from_path("middle", "1.0.0", "../m"));
        lock.sort();
        let names: Vec<&str> = lock.packages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "middle", "zebra"]);
    }

    #[test]
    fn empty_lock_file() {
        let lock = LockFile::new();
        assert_eq!(lock.lock_version, 1);
        assert!(lock.packages.is_empty());
        assert!(lock.find_package("nonexistent").is_none());
    }
}
