//! Project registry: holds loaded projects for source file I/O.

use std::collections::HashMap;
use std::path::PathBuf;

use sysml_project::{Project, ProjectHandle};

/// Registry of loaded projects, providing source file access.
#[derive(Default)]
pub struct ProjectRegistry {
    projects: HashMap<u32, Project>,
}

impl std::fmt::Debug for ProjectRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectRegistry")
            .field("projects", &self.projects.len())
            .finish()
    }
}

impl ProjectRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a project.
    pub fn register(&mut self, project: Project) {
        self.projects.insert(project.id.0, project);
    }

    /// Resolve a project-relative source file path to an on-disk path.
    pub fn source_path(&self, project_id: ProjectHandle, relative_path: &str) -> Option<PathBuf> {
        let project = self.projects.get(&project_id.0)?;
        match &project.root {
            sysml_project::ProjectRoot::Directory(dir) => Some(dir.join(relative_path)),
            sysml_project::ProjectRoot::InMemory => None,
            sysml_project::ProjectRoot::Kpar(_) => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_project::{ProjectHandle, ProjectInfo, ProjectRoot};

    #[test]
    fn source_path_resolves_directory_projects() {
        let dir = tempfile::tempdir().expect("temp dir should be created");
        let project = Project {
            id: ProjectHandle(42),
            info: ProjectInfo {
                name: "Disk".to_string(),
                description: None,
                version: "1.0.0".to_string(),
                topic: Vec::new(),
                usage: Vec::new(),
            },
            meta: None,
            root: ProjectRoot::Directory(dir.path().to_path_buf()),
        };

        let mut reg = ProjectRegistry::new();
        reg.register(project);
        let resolved = reg
            .source_path(ProjectHandle(42), "pkg/model.sysml")
            .expect("directory project path should resolve");
        assert_eq!(resolved, dir.path().join("pkg/model.sysml"));
    }
}
