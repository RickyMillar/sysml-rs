//! JSON schema types for `.project.json` and `.meta.json`.
//!
//! These match the OMG KerML clause 10.3 specification exactly.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Contents of `.project.json` inside a `.kpar` archive.
///
/// Example from the official Systems-Library.kpar:
/// ```json
/// {
///     "name": "SysML Systems Library",
///     "version": "2.0.0",
///     "description": "Standard semantic library for SysML",
///     "usage": [
///         {
///             "resource": "https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar",
///             "versionConstraint": "1.0.0"
///         }
///     ]
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectInfo {
    /// Project name.
    pub name: String,

    /// Project version (semver).
    pub version: String,

    /// Human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// License identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,

    /// Dependencies (called "usage" in the spec).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub usage: Vec<UsageEntry>,
}

/// A dependency declaration in `.project.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageEntry {
    /// Resource URL or IRI identifying the dependency.
    pub resource: String,

    /// Version constraint string.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "versionConstraint"
    )]
    pub version_constraint: Option<String>,
}

/// Contents of `.meta.json` inside a `.kpar` archive.
///
/// Example from the official Systems-Library.kpar:
/// ```json
/// {
///     "index": {
///         "Actions": "Actions.sysml",
///         "Parts": "Parts.sysml"
///     },
///     "created": "2025-03-13T00:00:00Z",
///     "metamodel": "https://www.omg.org/spec/SysML/20250201"
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectMetadata {
    /// Package name → source file mapping.
    #[serde(default)]
    pub index: BTreeMap<String, String>,

    /// Creation timestamp (ISO 8601).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,

    /// Metamodel specification URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metamodel: Option<String>,
}

impl ProjectInfo {
    /// Create a new ProjectInfo with required fields.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        ProjectInfo {
            name: name.into(),
            version: version.into(),
            description: None,
            license: None,
            usage: Vec::new(),
        }
    }

    /// Add a usage entry.
    pub fn add_usage(&mut self, resource: impl Into<String>, version_constraint: Option<String>) {
        self.usage.push(UsageEntry {
            resource: resource.into(),
            version_constraint,
        });
    }
}

impl ProjectMetadata {
    /// Create a new empty ProjectMetadata.
    pub fn new() -> Self {
        ProjectMetadata {
            index: BTreeMap::new(),
            created: None,
            metamodel: None,
        }
    }

    /// Set the creation timestamp to now (UTC, ISO 8601).
    pub fn set_created_now(&mut self) {
        self.created = Some(chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());
    }

    /// Add an index entry mapping a package name to a source file.
    pub fn add_index_entry(&mut self, package_name: impl Into<String>, file: impl Into<String>) {
        self.index.insert(package_name.into(), file.into());
    }
}

impl Default for ProjectMetadata {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn project_info_serialization() {
        let mut info = ProjectInfo::new("Test Library", "1.0.0");
        info.description = Some("A test library".to_string());
        info.add_usage(
            "https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar",
            Some("1.0.0".to_string()),
        );

        let json = serde_json::to_string_pretty(&info).unwrap();
        let parsed: ProjectInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, parsed);
    }

    #[test]
    fn project_info_matches_spec_format() {
        let json = r#"{
            "name": "SysML Systems Library",
            "version": "2.0.0",
            "description": "Standard semantic library for the Systems Modeling Language (SysML)",
            "usage": [
                {
                    "resource": "https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar",
                    "versionConstraint": "1.0.0"
                },
                {
                    "resource": "https://www.omg.org/spec/KerML/20250201/Data-Type-Library.kpar",
                    "versionConstraint": "1.0.0"
                }
            ]
        }"#;

        let info: ProjectInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.name, "SysML Systems Library");
        assert_eq!(info.version, "2.0.0");
        assert_eq!(info.usage.len(), 2);
        assert_eq!(
            info.usage[0].resource,
            "https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar"
        );
        assert_eq!(info.usage[0].version_constraint.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn metadata_serialization() {
        let mut meta = ProjectMetadata::new();
        meta.add_index_entry("Actions", "Actions.sysml");
        meta.add_index_entry("Parts", "Parts.sysml");
        meta.created = Some("2025-03-13T00:00:00Z".to_string());
        meta.metamodel = Some("https://www.omg.org/spec/SysML/20250201".to_string());

        let json = serde_json::to_string_pretty(&meta).unwrap();
        let parsed: ProjectMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(meta, parsed);
    }

    #[test]
    fn metadata_matches_spec_format() {
        let json = r#"{
            "index": {
                "Actions": "Actions.sysml",
                "Parts": "Parts.sysml",
                "States": "States.sysml"
            },
            "created": "2025-03-13T00:00:00Z",
            "metamodel": "https://www.omg.org/spec/SysML/20250201"
        }"#;

        let meta: ProjectMetadata = serde_json::from_str(json).unwrap();
        assert_eq!(meta.index.len(), 3);
        assert_eq!(meta.index["Actions"], "Actions.sysml");
        assert_eq!(meta.created.as_deref(), Some("2025-03-13T00:00:00Z"));
    }
}
