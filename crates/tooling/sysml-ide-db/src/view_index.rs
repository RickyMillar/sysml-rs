//! Tracked queries for `sysml_core::build_view_index`.
//!
//! `build_view_index(graph)` walks every element in a `ModelGraph`,
//! filters to `ViewUsage` / `ViewDefinition`, and summarises each one's
//! `Expose` / `ViewRenderingMembership` / `ElementFilterMembership`
//! children into a `ViewSummary`. Pure function of the elaborated
//! graph — the kind of work salsa is built to memoise.
//!
//! Caching closes one half of ADR-011 §3 / S3.T6. Each call against an
//! unchanged elaborated-graph revision returns the same
//! `Arc<Vec<ViewSummary>>`, so:
//!
//! - `service.views_list(uri)` and `service.views_render(uri, ...)` no
//!   longer re-walk the graph on every UI poll.
//! - `service.views_by_viewpoint(uri, vp_id)` (which today re-derives
//!   the index per call) can share the same cache.
//! - The FE-driven views panel sees a stable identity per revision —
//!   downstream cache layers can `Arc::ptr_eq` instead of comparing
//!   full vectors.
//!
//! The other half of T6 (caching compiled filter `ExprIR` per
//! `ElementFilterMembership`) lives in a sibling module and reuses the
//! same `(file / workspace / workspace+library)` shape.
//!
//! Three variants per query mirror `physics`, `signal_expr_table`,
//! `ref_resolve_cache`, `gated_expressions`, and the other tracked-
//! query modules in this crate:
//!
//! - `file_*` — single-file (no workspace, no library overlay).
//! - `workspace_*` — workspace-merged (no library overlay).
//! - `workspace_*_with_library` — workspace-merged with library overlay
//!   (the default for IDE / sim-app).

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_core::{build_view_index, ViewSummary};

use crate::analysis::{elaborate_workspace, elaborate_workspace_with_library};
use crate::parse;
use crate::project_inputs::ProjectFileSet;
use crate::resolution::LibraryGraph;
use crate::source::SourceFile;
use crate::Db;

/// Salsa-cached `Vec<ViewSummary>` derived from the elaborated graph.
///
/// Wraps `Arc<Vec<ViewSummary>>` with pointer-identity equality so
/// salsa can memoise the value cheaply — `ViewSummary` does not
/// implement `Hash` (it carries an `Option<Span>` whose path string
/// would be expensive to hash on every comparison), and identity
/// equality is sufficient because the value is only compared within
/// the same salsa revision.
#[derive(Clone, Debug)]
pub struct CachedViewIndex(Arc<Vec<ViewSummary>>);

impl CachedViewIndex {
    fn new(summaries: Vec<ViewSummary>) -> Self {
        Self(Arc::new(summaries))
    }

    /// Borrow the inner slice.
    pub fn summaries(&self) -> &[ViewSummary] {
        &self.0
    }

    /// Clone the inner `Arc<Vec<ViewSummary>>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<Vec<ViewSummary>> {
        Arc::clone(&self.0)
    }
}

salsa_arc_wrapper!(identity, CachedViewIndex, Vec<ViewSummary>);

// ---------------------------------------------------------------------------
// build_view_index — three variants
// ---------------------------------------------------------------------------

/// Build a view index for a single-file model graph.
///
/// Depends on: `parse_file()` (Layer 1).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_view_index(db: &dyn Db, source_file: SourceFile) -> CachedViewIndex {
    let parsed = parse::parse_file(db, source_file);
    let summaries = build_view_index(parsed.graph());
    CachedViewIndex::new(summaries)
}

/// Build a view index for a workspace-merged graph (no library overlay).
///
/// Depends on: `elaborate_workspace()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_view_index(db: &dyn Db, pfs: ProjectFileSet) -> CachedViewIndex {
    let elaborated = elaborate_workspace(db, pfs);
    let summaries = build_view_index(elaborated.graph());
    CachedViewIndex::new(summaries)
}

/// Build a view index for a workspace-merged graph with library overlay.
///
/// Depends on: `elaborate_workspace_with_library()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn workspace_view_index_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
) -> CachedViewIndex {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    let summaries = build_view_index(elaborated.graph());
    CachedViewIndex::new(summaries)
}

/// Best-shape dispatcher — Some(lib) routes to ..._with_library, None to bare workspace.
// TODO(post-collapse): inline dispatch-parity tests
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_view_index_best(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: Option<LibraryGraph>,
) -> CachedViewIndex {
    match library {
        Some(lib) => workspace_view_index_with_library(db, pfs, lib),
        None => workspace_view_index(db, pfs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::AnalysisHost;

    #[test]
    fn file_view_index_caches_across_calls() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def Engine; view def V { expose Engine; } }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let r1 = file_view_index(analysis.db(), sf);
        let r2 = file_view_index(analysis.db(), sf);

        assert!(Arc::ptr_eq(&r1.0, &r2.0));
    }

    #[test]
    fn file_view_index_invalidates_on_content_change() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { view def V1; }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");
        let r1 = file_view_index(host.analysis().db(), sf).arc();

        host.set_file_content(
            "test.sysml",
            "package P { view def V2; }".to_string(),
        );
        let sf2 = host.source_file(id).expect("source file still exists");
        let r2 = file_view_index(host.analysis().db(), sf2).arc();

        assert!(!Arc::ptr_eq(&r1, &r2));
    }

    #[test]
    fn file_view_index_returns_one_summary_per_view_def() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def A; view def V1 { expose A; } view def V2; }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");
        let r = file_view_index(host.analysis().db(), sf);
        // Two view-def elements → two summaries.
        assert_eq!(r.summaries().len(), 2);
    }
}
