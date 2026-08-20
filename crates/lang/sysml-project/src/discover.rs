use std::path::{Path, PathBuf};

use sysml_manifest::walk_up;

/// Result of project/workspace discovery.
#[derive(Debug, Clone)]
pub enum DiscoveryResult {
    /// Found a workspace at this path.
    Workspace(PathBuf),
    /// Found a standalone project at this path.
    Project(PathBuf),
    /// No project or workspace found.
    NotFound,
}

/// Walk up from `start_dir` looking for a `.workspace.json`.
///
/// Returns the directory containing the workspace file, or `NotFound`.
pub fn discover_workspace(start_dir: impl AsRef<Path>) -> DiscoveryResult {
    walk_up(start_dir.as_ref(), |dir| {
        if dir.join(".workspace.json").exists() {
            Some(DiscoveryResult::Workspace(dir.to_path_buf()))
        } else {
            None
        }
    })
    .unwrap_or(DiscoveryResult::NotFound)
}

/// Walk up from `start_dir` looking for a `.project.json` or `.workspace.json`.
///
/// Prefers `.workspace.json` if found first; otherwise returns the first
/// `.project.json` directory found.
pub fn discover_project(start_dir: impl AsRef<Path>) -> DiscoveryResult {
    walk_up(start_dir.as_ref(), |dir| {
        if dir.join(".workspace.json").exists() {
            Some(DiscoveryResult::Workspace(dir.to_path_buf()))
        } else if dir.join(".project.json").exists() {
            Some(DiscoveryResult::Project(dir.to_path_buf()))
        } else {
            None
        }
    })
    .unwrap_or(DiscoveryResult::NotFound)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn discover_in_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let result = discover_project(dir.path());
        assert!(matches!(result, DiscoveryResult::NotFound));
    }

    #[test]
    fn discover_project_in_current() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".project.json"),
            r#"{"name":"Test","version":"1.0.0"}"#,
        )
        .unwrap();

        match discover_project(dir.path()) {
            DiscoveryResult::Project(p) => assert_eq!(p, dir.path()),
            other => panic!("expected Project, got {other:?}"),
        }
    }

    #[test]
    fn discover_workspace_in_parent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".workspace.json"), r#"{"projects":[]}"#).unwrap();
        let subdir = dir.path().join("subproject");
        std::fs::create_dir(&subdir).unwrap();

        match discover_project(&subdir) {
            DiscoveryResult::Workspace(p) => assert_eq!(p, dir.path()),
            other => panic!("expected Workspace, got {other:?}"),
        }
    }

    #[test]
    fn discover_workspace_takes_priority() {
        let dir = tempfile::tempdir().unwrap();
        // Both workspace and project in same dir
        std::fs::write(dir.path().join(".workspace.json"), r#"{"projects":[]}"#).unwrap();
        std::fs::write(
            dir.path().join(".project.json"),
            r#"{"name":"Test","version":"1.0.0"}"#,
        )
        .unwrap();

        match discover_project(dir.path()) {
            DiscoveryResult::Workspace(p) => assert_eq!(p, dir.path()),
            other => panic!("expected Workspace, got {other:?}"),
        }
    }
}
