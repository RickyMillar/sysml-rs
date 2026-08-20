use std::path::{Path, PathBuf};

use crate::info::ProjectInfo;
use crate::meta::ProjectMeta;

/// Numeric project handle within a workspace session.
///
/// Distinct from `sysml_id::ProjectId` (the canonical content-derived id used
/// by salsa). This handle is a transient index into the `sysml-project`
/// registry — it lives only for the duration of a session and is not stable
/// across runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProjectHandle(pub u32);

/// Where a project's files are stored.
#[derive(Debug, Clone)]
pub enum ProjectRoot {
    /// A directory on the local filesystem.
    Directory(PathBuf),
    /// In-memory content (e.g. from embedded stdlib).
    InMemory,
    /// Inside a `.kpar` archive.
    #[cfg(feature = "kpar")]
    Kpar(PathBuf),
}

/// A loaded project with its manifest and metadata.
#[derive(Debug, Clone)]
pub struct Project {
    /// Unique ID within the current session.
    pub id: ProjectHandle,
    /// The project manifest.
    pub info: ProjectInfo,
    /// The project metadata (if available).
    pub meta: Option<ProjectMeta>,
    /// Where the project's source files live.
    pub root: ProjectRoot,
}

impl Project {
    /// Load a project from a directory containing `.project.json`.
    ///
    /// Optionally loads `.meta.json` if present.
    pub fn from_directory(id: ProjectHandle, dir: impl AsRef<Path>) -> crate::Result<Self> {
        let dir = dir.as_ref();
        let project_path = dir.join(".project.json");
        if !project_path.exists() {
            return Err(crate::Error::ProjectNotFound(dir.to_path_buf()));
        }

        let info = ProjectInfo::from_path(&project_path)?;

        let meta_path = dir.join(".meta.json");
        let meta = if meta_path.exists() {
            Some(ProjectMeta::from_path(&meta_path)?)
        } else {
            None
        };

        Ok(Self {
            id,
            info,
            meta,
            root: ProjectRoot::Directory(dir.to_path_buf()),
        })
    }

    /// Read a source file relative to the project root.
    pub fn read_source(&self, relative_path: &str) -> crate::Result<String> {
        match &self.root {
            ProjectRoot::Directory(dir) => {
                let path = dir.join(relative_path);
                Ok(std::fs::read_to_string(path)?)
            }
            ProjectRoot::InMemory => Err(crate::Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("in-memory project has no source files: {relative_path}"),
            ))),
            #[cfg(feature = "kpar")]
            ProjectRoot::Kpar(_) => Err(crate::Error::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "use KparReader to access kpar sources",
            ))),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn project_not_found() {
        let result = Project::from_directory(ProjectHandle(0), "/nonexistent/path");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, crate::Error::ProjectNotFound(_)));
    }
}
