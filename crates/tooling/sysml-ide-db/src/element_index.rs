//! Tracked queries for name and kind element indexes.
//!
//! Pre-S2.T17 the service `find` / `element` / `children` paths walked
//! `graph.elements.values()` linearly on every call — once per LSP
//! query, once per MCP `sysml_find` invocation, once per simulation
//! lookup. These tracked queries hoist that walk into salsa so it
//! collapses to a single compute per (input, library, parse) triple.
//!
//! Variants mirror `eval_context`:
//!
//! - `file_name_index(db, source_file)` — single-file mode
//! - `file_kind_index(db, source_file)` — single-file mode
//! - `workspace_name_index(db, pfs)` — workspace-merged (no library)
//! - `workspace_kind_index(db, pfs)` — workspace-merged (no library)
//! - `workspace_name_index_with_library(db, pfs, lib)` — workspace + lib
//! - `workspace_kind_index_with_library(db, pfs, lib)` — workspace + lib
//!
//! Result types `CachedNameIndex` / `CachedKindIndex` wrap
//! `Arc<HashMap<…>>` with identity-based equality (via
//! `salsa_arc_wrapper!(identity, …)`); salsa returns the same Arc on
//! cache hits so downstream consumers benefit transparently.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_core::{ElementId, ElementKind, ModelGraph};

use crate::analysis::{elaborate_workspace, elaborate_workspace_with_library};
use crate::parse;
use crate::project_inputs::ProjectFileSet;
use crate::resolution::LibraryGraph;
use crate::source::SourceFile;
use crate::Db;

/// Salsa-cached `name -> Vec<ElementId>` index.
///
/// Wraps `Arc<HashMap<String, Vec<ElementId>>>` with pointer-identity
/// equality so salsa can memoize the value across queries.
#[derive(Clone, Debug)]
pub struct CachedNameIndex(Arc<HashMap<String, Vec<ElementId>>>);

impl CachedNameIndex {
    fn new(map: HashMap<String, Vec<ElementId>>) -> Self {
        Self(Arc::new(map))
    }

    /// Borrow the inner map.
    pub fn map(&self) -> &HashMap<String, Vec<ElementId>> {
        &self.0
    }

    /// Clone the inner `Arc<HashMap<…>>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<HashMap<String, Vec<ElementId>>> {
        Arc::clone(&self.0)
    }

    /// Look up element IDs by exact name. Returns empty slice on miss.
    pub fn get(&self, name: &str) -> &[ElementId] {
        self.0.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

salsa_arc_wrapper!(identity, CachedNameIndex, HashMap<String, Vec<ElementId>>);

/// Salsa-cached `ElementKind -> Vec<ElementId>` index.
#[derive(Clone, Debug)]
pub struct CachedKindIndex(Arc<HashMap<ElementKind, Vec<ElementId>>>);

impl CachedKindIndex {
    fn new(map: HashMap<ElementKind, Vec<ElementId>>) -> Self {
        Self(Arc::new(map))
    }

    /// Borrow the inner map.
    pub fn map(&self) -> &HashMap<ElementKind, Vec<ElementId>> {
        &self.0
    }

    /// Clone the inner `Arc<HashMap<…>>`.
    pub fn arc(&self) -> Arc<HashMap<ElementKind, Vec<ElementId>>> {
        Arc::clone(&self.0)
    }

    /// Look up element IDs by exact kind. Returns empty slice on miss.
    pub fn get(&self, kind: &ElementKind) -> &[ElementId] {
        self.0.get(kind).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

salsa_arc_wrapper!(identity, CachedKindIndex, HashMap<ElementKind, Vec<ElementId>>);

fn build_name_index(graph: &ModelGraph) -> HashMap<String, Vec<ElementId>> {
    let mut map: HashMap<String, Vec<ElementId>> = HashMap::new();
    for el in graph.elements.values() {
        if let Some(name) = el.name.as_deref() {
            map.entry(name.to_owned()).or_default().push(el.id.clone());
        }
    }
    map
}

fn build_kind_index(graph: &ModelGraph) -> HashMap<ElementKind, Vec<ElementId>> {
    let mut map: HashMap<ElementKind, Vec<ElementId>> = HashMap::new();
    for el in graph.elements.values() {
        map.entry(el.kind.clone()).or_default().push(el.id.clone());
    }
    map
}

/// Build a name index for a single-file model graph.
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_name_index(db: &dyn Db, source_file: SourceFile) -> CachedNameIndex {
    let parsed = parse::parse_file(db, source_file);
    CachedNameIndex::new(build_name_index(parsed.graph()))
}

/// Build a kind index for a single-file model graph.
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_kind_index(db: &dyn Db, source_file: SourceFile) -> CachedKindIndex {
    let parsed = parse::parse_file(db, source_file);
    CachedKindIndex::new(build_kind_index(parsed.graph()))
}

/// Build a name index for a workspace-merged graph (no library overlay).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn workspace_name_index(db: &dyn Db, pfs: ProjectFileSet) -> CachedNameIndex {
    let elaborated = elaborate_workspace(db, pfs);
    CachedNameIndex::new(build_name_index(elaborated.graph()))
}

/// Build a kind index for a workspace-merged graph (no library overlay).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn workspace_kind_index(db: &dyn Db, pfs: ProjectFileSet) -> CachedKindIndex {
    let elaborated = elaborate_workspace(db, pfs);
    CachedKindIndex::new(build_kind_index(elaborated.graph()))
}

/// Build a name index for a workspace-merged graph with library overlay.
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_name_index_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
) -> CachedNameIndex {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    CachedNameIndex::new(build_name_index(elaborated.graph()))
}

/// Build a kind index for a workspace-merged graph with library overlay.
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_kind_index_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
) -> CachedKindIndex {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    CachedKindIndex::new(build_kind_index(elaborated.graph()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::AnalysisHost;

    #[test]
    fn file_name_index_caches_across_calls() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content("test.sysml", "package Foo {}".to_string());
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let idx1 = file_name_index(analysis.db(), sf);
        let idx2 = file_name_index(analysis.db(), sf);

        assert!(Arc::ptr_eq(&idx1.0, &idx2.0));
    }

    #[test]
    fn file_kind_index_caches_across_calls() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content("test.sysml", "package Foo {}".to_string());
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let idx1 = file_kind_index(analysis.db(), sf);
        let idx2 = file_kind_index(analysis.db(), sf);

        assert!(Arc::ptr_eq(&idx1.0, &idx2.0));
    }

    #[test]
    fn file_name_index_invalidates_on_content_change() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content("test.sysml", "package Foo {}".to_string());
        let sf = host.source_file(id).expect("source file exists");
        let idx1 = file_name_index(host.analysis().db(), sf).arc();

        host.set_file_content("test.sysml", "package Bar {}".to_string());
        let sf2 = host.source_file(id).expect("source file still exists");
        let idx2 = file_name_index(host.analysis().db(), sf2).arc();

        assert!(!Arc::ptr_eq(&idx1, &idx2));
    }

    #[test]
    fn file_name_index_finds_named_element() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content("test.sysml", "package Foo {}".to_string());
        let sf = host.source_file(id).expect("source file exists");

        let idx = file_name_index(host.analysis().db(), sf);
        let foo_ids = idx.get("Foo");
        assert!(
            !foo_ids.is_empty(),
            "expected at least one element named Foo, got {foo_ids:?}"
        );
    }

    #[test]
    fn file_kind_index_buckets_by_kind() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content("test.sysml", "package Foo {}".to_string());
        let sf = host.source_file(id).expect("source file exists");

        let idx = file_kind_index(host.analysis().db(), sf);
        let pkgs = idx.get(&ElementKind::Package);
        assert!(
            !pkgs.is_empty(),
            "expected at least one Package, got {pkgs:?}"
        );
    }
}
