//! Project and workspace discovery.
//!
//! Walks up the directory tree from a starting point to find `sysml.toml`
//! manifests and workspace roots, similar to how Cargo discovers `Cargo.toml`.

use std::path::{Path, PathBuf};

use crate::error::ManifestError;
use crate::manifest::{load_manifest, SysmlManifest, WorkspaceConfig};
use crate::path_walk::walk_up;
use crate::MANIFEST_FILENAME;

/// Search for a `sysml.toml` manifest by walking up from `start_dir`.
///
/// Returns the path to the manifest file and its parsed contents,
/// or `None` if no manifest is found before reaching the filesystem root.
pub fn find_manifest(start_dir: &Path) -> Result<Option<(PathBuf, SysmlManifest)>, ManifestError> {
    let start = start_dir
        .canonicalize()
        .map_err(|e| ManifestError::io(start_dir, e))?;

    walk_up(&start, |dir| {
        let manifest_path = dir.join(MANIFEST_FILENAME);
        if manifest_path.is_file() {
            Some(load_manifest(&manifest_path).map(|m| (manifest_path, m)))
        } else {
            None
        }
    })
    .transpose()
}

/// Search for a workspace root by walking up from `start_dir`.
///
/// A workspace root is a directory containing a `sysml.toml` with a
/// `[workspace]` section. Returns the path to the workspace manifest
/// and its workspace configuration, or `None` if no workspace is found.
pub fn find_workspace(
    start_dir: &Path,
) -> Result<Option<(PathBuf, WorkspaceConfig)>, ManifestError> {
    let start = start_dir
        .canonicalize()
        .map_err(|e| ManifestError::io(start_dir, e))?;

    walk_up(&start, |dir| {
        let manifest_path = dir.join(MANIFEST_FILENAME);
        if !manifest_path.is_file() {
            return None;
        }
        match load_manifest(&manifest_path) {
            Ok(manifest) => manifest
                .workspace
                .map(|ws| Ok((manifest_path, ws))),
            Err(e) => Some(Err(e)),
        }
    })
    .transpose()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "sysml-manifest-discovery-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn find_manifest_in_current_dir() {
        let root = temp_dir("current");
        let manifest_path = root.join(MANIFEST_FILENAME);
        fs::write(
            &manifest_path,
            r#"
[project]
name = "test"
version = "0.1.0"
"#,
        )
        .unwrap();

        let result = find_manifest(&root).unwrap();
        assert!(result.is_some());
        let (path, manifest) = result.unwrap();
        assert_eq!(
            path.canonicalize().unwrap(),
            manifest_path.canonicalize().unwrap()
        );
        assert_eq!(manifest.project.name, "test");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn find_manifest_walks_up() {
        let root = temp_dir("walkup");
        let subdir = root.join("src").join("models");
        fs::create_dir_all(&subdir).unwrap();

        let manifest_path = root.join(MANIFEST_FILENAME);
        fs::write(
            &manifest_path,
            r#"
[project]
name = "parent-project"
version = "1.0.0"
"#,
        )
        .unwrap();

        let result = find_manifest(&subdir).unwrap();
        assert!(result.is_some());
        let (_, manifest) = result.unwrap();
        assert_eq!(manifest.project.name, "parent-project");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn find_manifest_none_when_missing() {
        let root = temp_dir("nomatch");
        let result = find_manifest(&root).unwrap();
        assert!(result.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn find_workspace_root() {
        let root = temp_dir("workspace");
        let member_dir = root.join("member-a");
        fs::create_dir_all(&member_dir).unwrap();

        fs::write(
            root.join(MANIFEST_FILENAME),
            r#"
[project]
name = "my-workspace"
version = "0.1.0"

[workspace]
members = ["member-a", "member-b"]
"#,
        )
        .unwrap();

        // Non-workspace manifest in member
        fs::write(
            member_dir.join(MANIFEST_FILENAME),
            r#"
[project]
name = "member-a"
version = "0.1.0"
"#,
        )
        .unwrap();

        let result = find_workspace(&member_dir).unwrap();
        assert!(result.is_some());
        let (_, ws) = result.unwrap();
        assert_eq!(ws.members, vec!["member-a", "member-b"]);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn find_workspace_none_when_no_workspace() {
        let root = temp_dir("no-workspace");
        fs::write(
            root.join(MANIFEST_FILENAME),
            r#"
[project]
name = "standalone"
version = "0.1.0"
"#,
        )
        .unwrap();

        let result = find_workspace(&root).unwrap();
        assert!(result.is_none());

        let _ = fs::remove_dir_all(root);
    }
}
