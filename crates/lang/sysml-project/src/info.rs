use serde::{Deserialize, Serialize};

/// A project manifest (`.project.json`).
///
/// Matches the KerML Clause 10 schema for project interchange files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectInfo {
    /// Human-readable project name.
    pub name: String,

    /// Project description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Semantic version string (e.g. `"2.0.0"`).
    pub version: String,

    /// Topic tags (e.g. `["Standard"]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topic: Vec<String>,

    /// Dependencies on other projects, referenced by IRI.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub usage: Vec<ProjectUsage>,
}

/// A dependency reference within a `.project.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectUsage {
    /// IRI of the dependency (e.g. `"urn:kpar:semantic-library"`).
    pub resource: String,

    /// Version constraint string (e.g. `"1.0.0"`).
    pub version_constraint: String,
}

impl ProjectInfo {
    /// Parse a `ProjectInfo` from a JSON string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> crate::Result<Self> {
        Ok(serde_json::from_str(s)?)
    }

    /// Read a `ProjectInfo` from a `.project.json` file.
    pub fn from_path(path: impl AsRef<std::path::Path>) -> crate::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        Self::from_str(&contents)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_project_info() {
        let info = ProjectInfo {
            name: "Test Project".to_string(),
            description: Some("A test".to_string()),
            version: "1.0.0".to_string(),
            topic: vec!["Standard".to_string()],
            usage: vec![ProjectUsage {
                resource: "urn:kpar:semantic-library".to_string(),
                version_constraint: "1.0.0".to_string(),
            }],
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: ProjectInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, parsed);
    }

    #[test]
    fn parse_minimal_project() {
        let json = r#"{"name":"Minimal","version":"0.1.0"}"#;
        let info = ProjectInfo::from_str(json).unwrap();
        assert_eq!(info.name, "Minimal");
        assert!(info.description.is_none());
        assert!(info.topic.is_empty());
        assert!(info.usage.is_empty());
    }

    #[test]
    fn parse_systems_library_project() {
        let json = r#"{"name":"SysML Systems Library","description":"Standard semantic library for the Systems Modeling Language (SysML)","version":"2.0.0","topic":["Standard"],"usage":[{"resource":"urn:kpar:semantic-library","versionConstraint":"1.0.0"},{"resource":"urn:kpar:data-type-library","versionConstraint":"1.0.0"},{"resource":"urn:kpar:function-library","versionConstraint":"1.0.0"}]}"#;
        let info = ProjectInfo::from_str(json).unwrap();
        assert_eq!(info.name, "SysML Systems Library");
        assert_eq!(info.version, "2.0.0");
        assert_eq!(info.usage.len(), 3);
        assert_eq!(info.usage[0].resource, "urn:kpar:semantic-library");
        assert_eq!(info.usage[0].version_constraint, "1.0.0");
    }
}
