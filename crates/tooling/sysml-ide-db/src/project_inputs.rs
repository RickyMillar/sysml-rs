//! Salsa inputs for project-level metadata.
//!
//! These inputs represent project manifests and workspace configuration
//! within the salsa database. They enable incremental recomputation when
//! project metadata changes (e.g., new files added to a project's symbol index).

use std::sync::Arc;

use crate::source::SourceFile;
use sysml_project::{ProjectHandle, ProjectInfo, ProjectMeta};

/// A single project within the salsa database.
///
/// Each loaded project (including stdlib projects) gets one `SalsaProject`
/// input. When a project's metadata changes, all downstream queries that
/// depend on it are invalidated.
#[salsa::input(debug)]
pub struct SalsaProject {
    /// Numeric project ID (session-local).
    pub project_id: u32,
    /// Human-readable project name.
    #[returns(ref)]
    pub name: String,
    /// Parsed project manifest.
    #[returns(ref)]
    pub info: Arc<ProjectInfo>,
    /// Parsed project metadata (symbol index, checksums).
    #[returns(ref)]
    pub meta: Arc<ProjectMeta>,
}

impl SalsaProject {
    /// Get the `ProjectHandle` for this salsa project.
    pub fn pid(&self, db: &dyn crate::Db) -> ProjectHandle {
        ProjectHandle(self.project_id(db))
    }
}

/// Workspace-level configuration (singleton).
///
/// There is exactly one workspace per database instance. Resolution queries
/// take this as a parameter to explicitly declare the dependency on workspace
/// structure.
#[salsa::input(debug, singleton)]
pub struct WorkspaceConfig {
    /// All projects in the workspace (including stdlib if enabled).
    #[returns(ref)]
    pub projects: Arc<Vec<SalsaProject>>,
    /// Whether the standard library is included.
    pub include_stdlib: bool,
}

/// Project file set: maps ProjectHandle to its source files.
///
/// This is a salsa input that tracks which files belong to each project.
/// Used by workspace-aware resolution to find all files in a project.
///
/// The `kind` field carries `ProjectKind` as a `u8` (salsa needs Copy + Hash):
/// `0 = Discovered`, `1 = Strict`, `2 = DiscoveredViaManifest`. Use the
/// `PROJECT_KIND_*` constants below to avoid magic numbers.
#[salsa::input(debug)]
pub struct ProjectFileSet {
    /// The project ID.
    pub project_id: u32,
    /// All SourceFile inputs for this project.
    #[returns(ref)]
    pub files: Arc<Vec<SourceFile>>,
    /// How this project was discovered. Encoded as u8 because salsa
    /// requires Copy + Hash on input fields. See `PROJECT_KIND_*` constants.
    pub kind: u8,
}

/// Strict single-file (or synthetic) project — stdlib only, no folder context.
pub const PROJECT_KIND_STRICT: u8 = 1;
/// Folder opened, no `sysml.toml` found.
pub const PROJECT_KIND_DISCOVERED: u8 = 0;
/// `sysml.toml` at or above the opened root.
pub const PROJECT_KIND_DISCOVERED_VIA_MANIFEST: u8 = 2;

impl ProjectFileSet {
    /// Get the ProjectHandle for this file set.
    pub fn pid(&self, db: &dyn crate::Db) -> ProjectHandle {
        ProjectHandle(self.project_id(db))
    }

    /// True iff this project was opened in strict single-file mode.
    /// Triggers IM010 strict-flavour enrichment and IM012 emission in
    /// the diagnostic pipeline.
    pub fn is_strict(&self, db: &dyn crate::Db) -> bool {
        self.kind(db) == PROJECT_KIND_STRICT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RootDatabase;
    use std::collections::HashMap;

    fn make_test_info(name: &str) -> Arc<ProjectInfo> {
        Arc::new(ProjectInfo {
            name: name.to_string(),
            description: None,
            version: "1.0.0".to_string(),
            topic: Vec::new(),
            usage: Vec::new(),
        })
    }

    fn make_test_meta(symbols: &[(&str, &str)]) -> Arc<ProjectMeta> {
        let index: HashMap<String, String> = symbols
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Arc::new(ProjectMeta {
            index,
            created: None,
            metamodel: None,
            checksum: HashMap::new(),
        })
    }

    #[test]
    fn create_salsa_project() {
        let db = RootDatabase::default();
        let info = make_test_info("TestProject");
        let meta = make_test_meta(&[("Parts", "Parts.sysml")]);

        let proj = SalsaProject::new(&db, 0, "TestProject".to_string(), info, meta);
        assert_eq!(proj.name(&db), "TestProject");
        assert_eq!(proj.project_id(&db), 0);
        assert_eq!(proj.pid(&db), ProjectHandle(0));
    }

    #[test]
    fn create_workspace_config() {
        let db = RootDatabase::default();
        let info = make_test_info("MyProject");
        let meta = make_test_meta(&[("Foo", "Foo.sysml")]);
        let proj = SalsaProject::new(&db, 1, "MyProject".to_string(), info, meta);

        let config = WorkspaceConfig::new(&db, Arc::new(vec![proj]), false);
        assert!(!config.include_stdlib(&db));
        assert_eq!(config.projects(&db).len(), 1);
    }

    #[test]
    fn update_workspace_config() {
        let mut db = RootDatabase::default();
        let info1 = make_test_info("Proj1");
        let meta1 = make_test_meta(&[]);
        let p1 = SalsaProject::new(&db, 0, "Proj1".to_string(), info1, meta1);

        let config = WorkspaceConfig::new(&db, Arc::new(vec![p1]), false);
        assert_eq!(config.projects(&db).len(), 1);

        // Add a second project
        let info2 = make_test_info("Proj2");
        let meta2 = make_test_meta(&[("Bar", "Bar.sysml")]);
        let p2 = SalsaProject::new(&db, 1, "Proj2".to_string(), info2, meta2);

        use salsa::Setter;
        config.set_projects(&mut db).to(Arc::new(vec![p1, p2]));
        assert_eq!(config.projects(&db).len(), 2);
    }
}
