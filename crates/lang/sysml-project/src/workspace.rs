use serde::{Deserialize, Serialize};

/// A workspace manifest (`.workspace.json`).
///
/// Lists the projects contained within a workspace along with
/// their paths and IRI identifiers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    /// Projects in this workspace.
    pub projects: Vec<WorkspaceProject>,
}

/// A project entry within a workspace manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceProject {
    /// Relative path from the workspace root to the project directory.
    pub path: String,

    /// IRI identifiers for this project (e.g. `["urn:kpar:SysML-Systems-Library"]`).
    pub iris: Vec<String>,
}

impl WorkspaceInfo {
    /// Parse a `WorkspaceInfo` from a JSON string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> crate::Result<Self> {
        Ok(serde_json::from_str(s)?)
    }

    /// Read a `WorkspaceInfo` from a `.workspace.json` file.
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
    fn round_trip_workspace() {
        let ws = WorkspaceInfo {
            projects: vec![
                WorkspaceProject {
                    path: "Systems Library".to_string(),
                    iris: vec!["urn:kpar:SysML-Systems-Library".to_string()],
                },
                WorkspaceProject {
                    path: "Kernel Libraries/Kernel Semantic Library".to_string(),
                    iris: vec!["urn:kpar:Kernel-Semantic-Library".to_string()],
                },
            ],
        };
        let json = serde_json::to_string_pretty(&ws).unwrap();
        let parsed: WorkspaceInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(ws, parsed);
    }

    #[test]
    fn parse_pilot_workspace_format() {
        let json = r#"{
            "projects": [
                {"path": "Domain Libraries/Analysis", "iris": ["urn:kpar:SysML-Analysis-Library"]},
                {"path": "Systems Library", "iris": ["urn:kpar:SysML-Systems-Library"]}
            ]
        }"#;
        let ws = WorkspaceInfo::from_str(json).unwrap();
        assert_eq!(ws.projects.len(), 2);
        assert_eq!(ws.projects[0].path, "Domain Libraries/Analysis");
        assert_eq!(ws.projects[1].iris[0], "urn:kpar:SysML-Systems-Library");
    }
}
