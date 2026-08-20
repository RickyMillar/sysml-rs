//! Filesystem helpers for sysml-service.
//!
//! After S2.T3 deleted the parallel parse cache (`parse.rs`,
//! `parser_cache.rs`), the only fs-side helper that survived was the
//! `.sysml` discovery walk used by `from_workspace` / `load_workspace`.
//! It lives here so the service has a clear "fs operations" home and the
//! deleted `parse.rs` is genuinely gone.

use std::path::Path;

use crate::error::ServiceError;

/// One entry in a recursive workspace-file listing. Mirrors the REST
/// response shape from `sysml-api`'s old `/workspace/files` handler so the
/// transport layer collapses to a thin pass-through.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceFileEntry {
    pub name: String,
    pub path: String,
    /// Either `"file"` or `"directory"`.
    #[serde(rename = "type")]
    pub entry_type: String,
    /// `Some(children)` only for directories that contain SysML/KerML files.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<WorkspaceFileEntry>>,
}

/// Recursive payload returned by `list_workspace_files`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceFilesResult {
    pub root: String,
    pub entries: Vec<WorkspaceFileEntry>,
}

const DEFAULT_WORKSPACE_FILES_DEPTH: u32 = 5;

/// Recursively list `.sysml` / `.kerml` files under `root`, returning a
/// directory tree pruned to only directories that contain such files.
///
/// Skips dotfiles, `node_modules/`, `target/`, and `dist/`. The default
/// max depth (5) matches the previous REST handler. Returns
/// `ServiceError::Project` if `root` isn't a directory.
pub fn list_workspace_files(
    root: &Path,
    max_depth: Option<u32>,
) -> Result<WorkspaceFilesResult, ServiceError> {
    if !root.is_dir() {
        return Err(ServiceError::Project(format!(
            "not a directory: {}",
            root.display()
        )));
    }
    let depth = max_depth.unwrap_or(DEFAULT_WORKSPACE_FILES_DEPTH);
    let entries = scan_workspace_directory(root, depth)?;
    Ok(WorkspaceFilesResult {
        root: root.to_string_lossy().into_owned(),
        entries,
    })
}

fn scan_workspace_directory(
    dir: &Path,
    max_depth: u32,
) -> Result<Vec<WorkspaceFileEntry>, ServiceError> {
    if max_depth == 0 {
        return Ok(Vec::new());
    }
    let mut dir_entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| ServiceError::io(dir, e))?
        .filter_map(|e| e.ok())
        .collect();
    dir_entries.sort_by_key(|e| e.file_name());

    let mut result = Vec::new();
    for entry in dir_entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "node_modules" || name == "target" || name == "dist" {
            continue;
        }
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_dir() {
            let children = scan_workspace_directory(&path, max_depth - 1)?;
            if !children.is_empty() {
                result.push(WorkspaceFileEntry {
                    name,
                    path: path.to_string_lossy().into_owned(),
                    entry_type: "directory".to_owned(),
                    children: Some(children),
                });
            }
        } else if ft.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "sysml" || ext == "kerml" {
                result.push(WorkspaceFileEntry {
                    name,
                    path: path.to_string_lossy().into_owned(),
                    entry_type: "file".to_owned(),
                    children: None,
                });
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn list_workspace_files_includes_directories_with_sysml_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let nested_sub = sub.join("deep");
        std::fs::create_dir(&nested_sub).unwrap();
        let empty_sub = dir.path().join("empty");
        std::fs::create_dir(&empty_sub).unwrap();
        let hidden = dir.path().join(".hidden");
        std::fs::create_dir(&hidden).unwrap();

        std::fs::File::create(dir.path().join("top.sysml"))
            .unwrap()
            .write_all(b"")
            .unwrap();
        std::fs::File::create(dir.path().join("ignored.txt"))
            .unwrap()
            .write_all(b"")
            .unwrap();
        std::fs::File::create(sub.join("nested.kerml"))
            .unwrap()
            .write_all(b"")
            .unwrap();
        std::fs::File::create(nested_sub.join("deeper.sysml"))
            .unwrap()
            .write_all(b"")
            .unwrap();
        // hidden file should be skipped
        std::fs::File::create(hidden.join("ghost.sysml"))
            .unwrap()
            .write_all(b"")
            .unwrap();

        let result = list_workspace_files(dir.path(), None).unwrap();

        // Directories without SysML files should be pruned.
        assert!(!result.entries.iter().any(|e| e.name == "empty"));
        // Hidden directories should be skipped.
        assert!(!result.entries.iter().any(|e| e.name == ".hidden"));
        // top.sysml should be present at the top level.
        assert!(result
            .entries
            .iter()
            .any(|e| e.name == "top.sysml" && e.entry_type == "file"));
        // sub/ should be present and contain its own children.
        let sub_entry = result
            .entries
            .iter()
            .find(|e| e.name == "sub")
            .expect("sub directory entry");
        assert_eq!(sub_entry.entry_type, "directory");
        let sub_children = sub_entry.children.as_ref().expect("sub children");
        assert!(sub_children.iter().any(|c| c.name == "nested.kerml"));
        assert!(sub_children.iter().any(|c| c.name == "deep"));
    }

    #[test]
    fn list_workspace_files_rejects_non_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.sysml");
        std::fs::File::create(&file).unwrap().write_all(b"").unwrap();
        let err = list_workspace_files(&file, None).unwrap_err();
        assert!(matches!(err, ServiceError::Project(_)));
    }

    #[test]
    fn list_workspace_files_max_depth_zero_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::File::create(dir.path().join("top.sysml"))
            .unwrap()
            .write_all(b"")
            .unwrap();
        let result = list_workspace_files(dir.path(), Some(0)).unwrap();
        assert!(result.entries.is_empty());
    }
}
