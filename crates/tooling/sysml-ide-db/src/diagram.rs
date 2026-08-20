//! Tracked queries for `sysml_diagram::smodel::to_smodel_with`.
//!
//! Closes ADR-011 §3 / S3.T5. `to_smodel_with(graph, &ViewRequest)`
//! is the canonical SGraph generator behind every "render this view"
//! call (LSP `setModel`, service `diagram_with`, every diagram
//! command). It walks the graph, classifies elements per the
//! `ViewType`-specific generator, builds a `DiagramIR`, then renders
//! to a `SGraph`. None of that depends on session state — it's a pure
//! function of `(graph, request)`. Salsa-cacheable.
//!
//! The cache key is the [`DiagramRequestKey`] newtype from
//! `sysml-diagram` (S3.T4), which restricts the key to
//! `(view_type, expanded_ids, expose)` — the LSP `generate_diagram`
//! shape. Requests carrying `filter` / `hints` / `overlays`
//! deliberately bypass the cache: those fields appear only on
//! user-authored `ViewUsage` requests and resolve differently per
//! filter-expression compilation (which S3.T6b will cache
//! separately).
//!
//! Three variants per query mirror `view_index`, `physics`,
//! `signal_expr_table`, and the other tracked-query modules in this
//! crate:
//!
//! - `file_*` — single-file (no workspace, no library overlay).
//! - `workspace_*` — workspace-merged (no library overlay).
//! - `workspace_*_with_library` — workspace-merged with library overlay
//!   (the default for IDE / sim-app).

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_diagram::smodel::{to_smodel_with, SGraph};
use sysml_diagram::DiagramRequestKey;

use crate::analysis::{elaborate_workspace, elaborate_workspace_with_library};
use crate::parse;
use crate::project_inputs::ProjectFileSet;
use crate::resolution::LibraryGraph;
use crate::source::SourceFile;
use crate::Db;

/// Salsa-cached `SGraph` produced by `to_smodel_with` for a stable
/// [`DiagramRequestKey`] against an unchanged elaborated graph.
///
/// Wraps `Arc<SGraph>` with pointer-identity equality. `SGraph` does
/// not implement `Eq` or `Hash` (it's a deep tree of enum variants
/// carrying ELK layout strings, CSS classes, semantic-element ids
/// — hashing the whole structure each time would defeat the
/// purpose of caching). Identity equality is sufficient because the
/// value is only compared within the same salsa revision.
#[derive(Clone, Debug)]
pub struct CachedDiagram(Arc<SGraph>);

impl CachedDiagram {
    fn new(sgraph: SGraph) -> Self {
        Self(Arc::new(sgraph))
    }

    /// Borrow the inner `SGraph`.
    pub fn sgraph(&self) -> &SGraph {
        &self.0
    }

    /// Clone the inner `Arc<SGraph>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<SGraph> {
        Arc::clone(&self.0)
    }
}

salsa_arc_wrapper!(identity, CachedDiagram, SGraph);

// ---------------------------------------------------------------------------
// to_smodel_with — three variants
// ---------------------------------------------------------------------------

/// Generate an SGraph for a single-file model graph.
///
/// Depends on: `parse_file()` (Layer 1).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_diagram(
    db: &dyn Db,
    source_file: SourceFile,
    key: DiagramRequestKey,
) -> CachedDiagram {
    let parsed = parse::parse_file(db, source_file);
    let request = key.to_view_request();
    let sgraph = to_smodel_with(parsed.graph(), &request);
    CachedDiagram::new(sgraph)
}

/// Generate an SGraph for a workspace-merged graph (no library overlay).
///
/// Depends on: `elaborate_workspace()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_diagram(
    db: &dyn Db,
    pfs: ProjectFileSet,
    key: DiagramRequestKey,
) -> CachedDiagram {
    let elaborated = elaborate_workspace(db, pfs);
    let request = key.to_view_request();
    let sgraph = to_smodel_with(elaborated.graph(), &request);
    CachedDiagram::new(sgraph)
}

/// Generate an SGraph for a workspace-merged graph with library overlay.
///
/// Depends on: `elaborate_workspace_with_library()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn workspace_diagram_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
    key: DiagramRequestKey,
) -> CachedDiagram {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    let request = key.to_view_request();
    let sgraph = to_smodel_with(elaborated.graph(), &request);
    CachedDiagram::new(sgraph)
}

/// Best-shape dispatcher — Some(lib) routes to ..._with_library, None to bare workspace.
// TODO(post-collapse): inline dispatch-parity tests
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_diagram_best(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: Option<LibraryGraph>,
    key: DiagramRequestKey,
) -> CachedDiagram {
    match library {
        Some(lib) => workspace_diagram_with_library(db, pfs, lib, key),
        None => workspace_diagram(db, pfs, key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::AnalysisHost;
    use sysml_diagram::smodel::ViewType;
    use sysml_diagram::ViewRequest;

    fn key_for(vt: ViewType) -> DiagramRequestKey {
        ViewRequest::new(vt).cache_key().expect("plain request is cacheable")
    }

    #[test]
    fn file_diagram_caches_across_calls() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def Engine; }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");
        let key = key_for(ViewType::General);

        let analysis = host.analysis();
        let r1 = file_diagram(analysis.db(), sf, key.clone());
        let r2 = file_diagram(analysis.db(), sf, key);

        assert!(Arc::ptr_eq(&r1.0, &r2.0));
    }

    #[test]
    fn file_diagram_invalidates_on_content_change() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def A; }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");
        let key = key_for(ViewType::General);
        let r1 = file_diagram(host.analysis().db(), sf, key.clone()).arc();

        host.set_file_content(
            "test.sysml",
            "package P { part def B; }".to_string(),
        );
        let sf2 = host.source_file(id).expect("source file still exists");
        let r2 = file_diagram(host.analysis().db(), sf2, key).arc();

        assert!(!Arc::ptr_eq(&r1, &r2));
    }

    #[test]
    fn file_diagram_differs_by_view_type() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def Engine; }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let general = file_diagram(host.analysis().db(), sf, key_for(ViewType::General));
        let interconnection = file_diagram(
            host.analysis().db(),
            sf,
            key_for(ViewType::Interconnection),
        );

        // Distinct cache slots → distinct Arcs.
        assert!(!Arc::ptr_eq(&general.0, &interconnection.0));
    }
}
