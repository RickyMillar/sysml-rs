use serde::{Deserialize, Serialize};

/// A lock file describing the resolved project dependency graph.
///
/// Format is TOML, compatible with `sysand-lock.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KparLockFile {
    /// Schema version for the lock file format.
    #[serde(default = "default_version")]
    pub version: u32,

    /// Resolved projects in dependency order.
    #[serde(default, rename = "project")]
    pub projects: Vec<LockedProject>,
}

fn default_version() -> u32 {
    1
}

/// A resolved project entry in the lock file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockedProject {
    /// Project name.
    pub name: String,

    /// Resolved version.
    pub version: String,

    /// Where the project comes from.
    pub source: ProjectSource,

    /// Resolved dependency names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,

    /// SHA-256 checksum of the resolved content (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

/// Where a locked project's content comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "type")]
pub enum ProjectSource {
    /// A local directory path.
    Path { path: String },
    /// A `.kpar` archive.
    Kpar { path: String },
    /// An embedded standard library project (resolved by IRI).
    Stdlib { iri: String },
}

impl KparLockFile {
    /// Parse a lock file from a TOML string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> crate::Result<Self> {
        Ok(toml::from_str(s)?)
    }

    /// Read a lock file from a path.
    pub fn from_path(path: impl AsRef<std::path::Path>) -> crate::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        Self::from_str(&contents)
    }

    /// Serialize to a pretty TOML string.
    pub fn to_string_pretty(&self) -> crate::Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_lock_file() {
        let lock = KparLockFile {
            version: 1,
            projects: vec![
                LockedProject {
                    name: "Kernel Semantic Library".to_string(),
                    version: "1.0.0".to_string(),
                    source: ProjectSource::Stdlib {
                        iri: "urn:kpar:semantic-library".to_string(),
                    },
                    dependencies: vec![
                        "Kernel Data Type Library".to_string(),
                        "Kernel Function Library".to_string(),
                    ],
                    checksum: None,
                },
                LockedProject {
                    name: "My Project".to_string(),
                    version: "0.1.0".to_string(),
                    source: ProjectSource::Path {
                        path: ".".to_string(),
                    },
                    dependencies: vec!["Kernel Semantic Library".to_string()],
                    checksum: None,
                },
            ],
        };

        let toml_str = lock.to_string_pretty().unwrap();
        let parsed = KparLockFile::from_str(&toml_str).unwrap();
        assert_eq!(lock, parsed);
    }

    #[test]
    fn parse_minimal_lock() {
        let toml_str = "version = 1\n";
        let lock = KparLockFile::from_str(toml_str).unwrap();
        assert_eq!(lock.version, 1);
        assert!(lock.projects.is_empty());
    }

    #[test]
    fn lock_with_kpar_source() {
        let lock = KparLockFile {
            version: 1,
            projects: vec![LockedProject {
                name: "Systems Library".to_string(),
                version: "2.0.0".to_string(),
                source: ProjectSource::Kpar {
                    path: "libs/Systems-Library.kpar".to_string(),
                },
                dependencies: vec![],
                checksum: Some("abc123".to_string()),
            }],
        };
        let toml_str = lock.to_string_pretty().unwrap();
        assert!(toml_str.contains("type = \"kpar\""));
        let parsed = KparLockFile::from_str(&toml_str).unwrap();
        assert_eq!(lock, parsed);
    }
}
