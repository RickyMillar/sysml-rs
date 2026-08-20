//! Symbol index queries: build a cross-project symbol lookup table.
//!
//! The `symbol_index` tracked query builds a global [`SymbolIndex`] from all
//! project metadata in the workspace. It recomputes only when the
//! `WorkspaceConfig` or any `SalsaProject`'s metadata changes.
//!
//! File content is NOT a dependency — adding/parsing files doesn't invalidate
//! the index.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_project::{ProjectHandle, SymbolIndex};

use crate::project_inputs::WorkspaceConfig;
use crate::Db;

/// Result wrapper for the symbol index (salsa-compatible).
#[derive(Clone, Debug)]
pub struct GlobalSymbolIndex(Arc<GlobalSymbolIndexData>);

#[derive(Debug)]
struct GlobalSymbolIndexData {
    index: SymbolIndex,
    fingerprint: u64,
}

impl GlobalSymbolIndex {
    fn new(index: SymbolIndex) -> Self {
        let fingerprint = {
            use std::collections::hash_map::DefaultHasher;
            let mut h = DefaultHasher::new();
            index.len().hash(&mut h);
            for entry in index.iter() {
                entry.symbol.hash(&mut h);
                entry.file.hash(&mut h);
                entry.project.hash(&mut h);
            }
            h.finish()
        };
        Self(Arc::new(GlobalSymbolIndexData { index, fingerprint }))
    }

    /// The symbol index.
    pub fn index(&self) -> &SymbolIndex {
        &self.0.index
    }

    /// Number of symbols in the index.
    pub fn len(&self) -> usize {
        self.0.index.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.0.index.is_empty()
    }
}

salsa_arc_wrapper!(fingerprint, GlobalSymbolIndex, GlobalSymbolIndexData);

/// Build the global symbol index from all projects in the workspace.
///
/// This tracked query depends on `WorkspaceConfig` (which transitively depends
/// on each `SalsaProject`'s metadata). It recomputes only when project metadata
/// changes — not when file contents change.
#[salsa::tracked]
pub fn symbol_index(db: &dyn Db, config: WorkspaceConfig) -> GlobalSymbolIndex {
    let projects = config.projects(db);
    let mut combined = SymbolIndex::new();

    for salsa_proj in projects.iter() {
        let name = salsa_proj.name(db);
        let meta = salsa_proj.meta(db);
        let proj_index = meta.symbol_index(name);
        combined.merge(&proj_index);
    }

    GlobalSymbolIndex::new(combined)
}

/// Look up a symbol across all projects in the workspace.
///
/// Returns the project ID and source file path if found.
pub fn resolve_symbol(
    db: &dyn Db,
    config: WorkspaceConfig,
    name: &str,
) -> Option<(ProjectHandle, String)> {
    let idx = symbol_index(db, config);
    let entries = idx.index().lookup(name);
    // Return the first match (caller can handle ambiguity via conflicts())
    entries.first().map(|entry| {
        // Find the project ID by matching the project name
        let projects = config.projects(db);
        let pid = projects
            .iter()
            .find(|p| *p.name(db) == entry.project)
            .map(|p| p.pid(db))
            .unwrap_or(ProjectHandle(0));
        (pid, entry.file.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_inputs::SalsaProject;
    use crate::RootDatabase;
    use std::collections::HashMap;
    use sysml_project::{ProjectInfo, ProjectMeta};

    fn make_info(name: &str) -> Arc<ProjectInfo> {
        Arc::new(ProjectInfo {
            name: name.to_string(),
            description: None,
            version: "1.0.0".to_string(),
            topic: Vec::new(),
            usage: Vec::new(),
        })
    }

    fn make_meta(symbols: &[(&str, &str)]) -> Arc<ProjectMeta> {
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
    fn symbol_index_builds_from_projects() {
        let db = RootDatabase::default();

        let p1 = SalsaProject::new(
            &db,
            0,
            "ProjectA".to_string(),
            make_info("ProjectA"),
            make_meta(&[("Parts", "Parts.sysml"), ("Actions", "Actions.sysml")]),
        );
        let p2 = SalsaProject::new(
            &db,
            1,
            "ProjectB".to_string(),
            make_info("ProjectB"),
            make_meta(&[("Base", "Base.kerml")]),
        );

        let config = WorkspaceConfig::new(&db, Arc::new(vec![p1, p2]), false);
        let idx = symbol_index(&db, config);

        assert_eq!(idx.len(), 3);
        assert!(!idx.index().lookup("Parts").is_empty());
        assert!(!idx.index().lookup("Base").is_empty());
        assert!(idx.index().lookup("Missing").is_empty());
    }

    #[test]
    fn resolve_symbol_finds_entry() {
        let db = RootDatabase::default();

        let p1 = SalsaProject::new(
            &db,
            0,
            "MyLib".to_string(),
            make_info("MyLib"),
            make_meta(&[("Widgets", "Widgets.sysml")]),
        );

        let config = WorkspaceConfig::new(&db, Arc::new(vec![p1]), false);
        let result = resolve_symbol(&db, config, "Widgets");

        assert!(result.is_some());
        let (pid, file) = result.unwrap();
        assert_eq!(pid, ProjectHandle(0));
        assert_eq!(file, "Widgets.sysml");
    }

    #[test]
    fn resolve_symbol_returns_none_for_missing() {
        let db = RootDatabase::default();
        let config = WorkspaceConfig::new(&db, Arc::new(vec![]), false);
        assert!(resolve_symbol(&db, config, "Nonexistent").is_none());
    }

    #[test]
    fn symbol_index_recomputes_on_meta_change() {
        let mut db = RootDatabase::default();

        let meta1 = make_meta(&[("Foo", "Foo.sysml")]);
        let p1 = SalsaProject::new(&db, 0, "P".to_string(), make_info("P"), meta1);
        let config = WorkspaceConfig::new(&db, Arc::new(vec![p1]), false);

        let idx1 = symbol_index(&db, config);
        assert_eq!(idx1.len(), 1);

        // Update meta to add a new symbol
        let meta2 = make_meta(&[("Foo", "Foo.sysml"), ("Bar", "Bar.sysml")]);
        use salsa::Setter;
        p1.set_meta(&mut db).to(meta2);

        let idx2 = symbol_index(&db, config);
        assert_eq!(idx2.len(), 2);
        assert_ne!(idx1, idx2);
    }

    #[test]
    fn empty_workspace_produces_empty_index() {
        let db = RootDatabase::default();
        let config = WorkspaceConfig::new(&db, Arc::new(vec![]), true);
        let idx = symbol_index(&db, config);
        assert!(idx.is_empty());
    }
}
