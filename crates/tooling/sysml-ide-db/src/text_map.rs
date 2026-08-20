//! Tracked queries for the `ElementId↔Span` text-map (Bucket 1.6).
//!
//! The text-map (`sysml_diagram::build_text_map`) projects each element's primary
//! source span into a compact, node-id-keyed form for the bidirectional
//! text↔diagram link. Unlike the diagram / view-model queries it is **not** keyed
//! by a view request — it is a property of the model graph, so it is built once
//! per `(graph)` and shared across every view (the cheaper standalone query the
//! `ViewModel` holds a ref into). Pure function of the elaborated graph.
//!
//! Three variants mirror the rest of the crate:
//! - `file_*` — single-file.
//! - `workspace_*` — workspace-merged (no library overlay).
//! - `workspace_*_with_library` — workspace-merged with library overlay.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_diagram::{build_text_map, TextMap};

use crate::analysis::{elaborate_workspace, elaborate_workspace_with_library};
use crate::parse;
use crate::project_inputs::ProjectFileSet;
use crate::resolution::LibraryGraph;
use crate::source::SourceFile;
use crate::Db;

/// Salsa-cached [`TextMap`] for an unchanged elaborated graph.
///
/// Wraps `Arc<TextMap>` with pointer-identity equality (same rationale as
/// [`crate::diagram::CachedDiagram`]).
#[derive(Clone, Debug)]
pub struct CachedTextMap(Arc<TextMap>);

impl CachedTextMap {
    fn new(text_map: TextMap) -> Self {
        Self(Arc::new(text_map))
    }

    /// Borrow the inner [`TextMap`].
    pub fn text_map(&self) -> &TextMap {
        &self.0
    }

    /// Clone the inner `Arc<TextMap>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<TextMap> {
        Arc::clone(&self.0)
    }
}

salsa_arc_wrapper!(identity, CachedTextMap, TextMap);

/// Text-map for a single-file model graph. Depends on `parse_file()` (Layer 1).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_text_map(db: &dyn Db, source_file: SourceFile) -> CachedTextMap {
    let parsed = parse::parse_file(db, source_file);
    CachedTextMap::new(build_text_map(parsed.graph()))
}

/// Text-map for a workspace-merged graph (no library overlay).
/// Depends on `elaborate_workspace()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_text_map(db: &dyn Db, pfs: ProjectFileSet) -> CachedTextMap {
    let elaborated = elaborate_workspace(db, pfs);
    CachedTextMap::new(build_text_map(elaborated.graph()))
}

/// Text-map for a workspace-merged graph with library overlay.
/// Depends on `elaborate_workspace_with_library()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn workspace_text_map_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
) -> CachedTextMap {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    CachedTextMap::new(build_text_map(elaborated.graph()))
}

/// Best-shape dispatcher — `Some(lib)` routes to `..._with_library`, `None` to
/// the bare workspace query.
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_text_map_best(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: Option<LibraryGraph>,
) -> CachedTextMap {
    match library {
        Some(lib) => workspace_text_map_with_library(db, pfs, lib),
        None => workspace_text_map(db, pfs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::AnalysisHost;

    #[test]
    fn file_text_map_caches_and_maps_elements() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def Engine; }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let r1 = file_text_map(analysis.db(), sf);
        let r2 = file_text_map(analysis.db(), sf);
        assert!(Arc::ptr_eq(&r1.0, &r2.0), "should memoize");
        assert!(!r1.text_map().is_empty(), "should map the model's elements");
    }

    #[test]
    fn file_text_map_invalidates_on_content_change() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def A; }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");
        let r1 = file_text_map(host.analysis().db(), sf).arc();

        host.set_file_content(
            "test.sysml",
            "package P { part def A; part def B; }".to_string(),
        );
        let sf2 = host.source_file(id).expect("source file still exists");
        let r2 = file_text_map(host.analysis().db(), sf2).arc();

        assert!(!Arc::ptr_eq(&r1, &r2));
    }
}
