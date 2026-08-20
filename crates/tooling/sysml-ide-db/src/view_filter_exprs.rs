//! Tracked queries for precompiled `ElementFilterMembership` ExprIRs.
//!
//! Closes ADR-011 §3 / S3.T6b — the second half of T6. The first
//! half (`view_index`) caches the discovery of user-authored views;
//! this half caches the compilation of each view's filter
//! expression so diagram rendering with `filter` set doesn't
//! re-parse / re-compile the same `ElementFilterMembership` body on
//! every candidate element.
//!
//! Per-element filter evaluation runs inside the diagram generator's
//! `passes_filter` (sysml-diagram::GeneratorContext) — invoked once
//! per element collected for the top-level canvas, which on a
//! workspace-scale graph is thousands of calls per render. Compiling
//! the same filter ExprIR thousands of times per render is wasteful;
//! one compile per `(filter_membership_id, revision)` is correct.
//!
//! The cache is shaped as `Arc<HashMap<ElementId, Arc<ExprIR>>>`,
//! keyed by the `ElementFilterMembership` element id. Walks every
//! element in the elaborated graph, identifies filter memberships,
//! and stores the result of
//! [`sysml_runtime::view_condition::resolve_filter_expr_ir`].
//! Filter holders whose expression doesn't compile are skipped
//! (the runtime's safe fall-through evaluates them as `true`).
//!
//! Three variants per query mirror `view_index`, `diagram`,
//! `physics`, and the other tracked-query modules:
//!
//! - `file_*` — single-file (no workspace, no library overlay).
//! - `workspace_*` — workspace-merged (no library overlay).
//! - `workspace_*_with_library` — workspace-merged with library overlay.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_core::{ElementId, ElementKind, ModelGraph};
use sysml_runtime::expressions::ExprIR;
use sysml_runtime::view_condition::resolve_filter_expr_ir;

use crate::analysis::{elaborate_workspace, elaborate_workspace_with_library};
use crate::parse;
use crate::project_inputs::ProjectFileSet;
use crate::resolution::LibraryGraph;
use crate::source::SourceFile;
use crate::Db;

/// Precompiled `ElementFilterMembership` ExprIRs for one graph
/// revision. Maps `ElementFilterMembership` element id →
/// `Arc<ExprIR>` (so multiple `GeneratorContext`s with overlapping
/// filter sets share storage).
pub type ViewFilterExprMap = HashMap<ElementId, Arc<ExprIR>>;

/// Salsa-cached precompiled-filter map.
///
/// Wraps `Arc<ViewFilterExprMap>` with pointer-identity equality —
/// `ExprIR` doesn't implement `Eq` / `Hash` (it's a recursive enum
/// of expression nodes whose hash would dominate the cache benefit).
/// Identity equality is sufficient because the value is only
/// compared within the same salsa revision.
#[derive(Clone, Debug)]
pub struct CachedViewFilterExprs(Arc<ViewFilterExprMap>);

impl CachedViewFilterExprs {
    fn new(map: ViewFilterExprMap) -> Self {
        Self(Arc::new(map))
    }

    /// Borrow the inner map.
    pub fn map(&self) -> &ViewFilterExprMap {
        &self.0
    }

    /// Clone the inner `Arc<ViewFilterExprMap>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<ViewFilterExprMap> {
        Arc::clone(&self.0)
    }

    /// Look up a precompiled `ExprIR` for an `ElementFilterMembership`.
    pub fn get(&self, filter_id: &ElementId) -> Option<&Arc<ExprIR>> {
        self.0.get(filter_id)
    }
}

salsa_arc_wrapper!(identity, CachedViewFilterExprs, ViewFilterExprMap);

/// Walk every `ElementFilterMembership` in `graph` and compile its
/// expression to `ExprIR`. Holders whose expression fails to compile
/// are skipped — the safe fall-through evaluator treats missing
/// entries as `true`, matching the existing `evaluate_view_condition`
/// behaviour.
fn build_filter_expr_map(graph: &ModelGraph) -> ViewFilterExprMap {
    let mut out = ViewFilterExprMap::new();
    for element in graph.elements.values() {
        if !matches!(element.kind, ElementKind::ElementFilterMembership) {
            continue;
        }
        if let Some(ir) = resolve_filter_expr_ir(graph, element) {
            out.insert(element.id.clone(), Arc::new(ir));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// view_filter_exprs — three variants
// ---------------------------------------------------------------------------

/// Precompile filter ExprIRs for a single-file model graph.
///
/// Depends on: `parse_file()` (Layer 1).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn file_view_filter_exprs(
    db: &dyn Db,
    source_file: SourceFile,
) -> CachedViewFilterExprs {
    let parsed = parse::parse_file(db, source_file);
    CachedViewFilterExprs::new(build_filter_expr_map(parsed.graph()))
}

/// Precompile filter ExprIRs for a workspace-merged graph (no library).
///
/// Depends on: `elaborate_workspace()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_view_filter_exprs(
    db: &dyn Db,
    pfs: ProjectFileSet,
) -> CachedViewFilterExprs {
    let elaborated = elaborate_workspace(db, pfs);
    CachedViewFilterExprs::new(build_filter_expr_map(elaborated.graph()))
}

/// Precompile filter ExprIRs for a workspace-merged graph with library overlay.
///
/// Depends on: `elaborate_workspace_with_library()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn workspace_view_filter_exprs_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
) -> CachedViewFilterExprs {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    CachedViewFilterExprs::new(build_filter_expr_map(elaborated.graph()))
}

/// Three-arm dispatcher for the precompiled-filter cache.
///
/// Mirrors [`resolution::resolve_file_best`] in shape — caller passes the
/// host's optional [`ProjectFileSet`] + [`LibraryGraph`] and the dispatcher
/// picks the strongest available arm.
///
/// | `project_files` | `library`  | Strategy                                               |
/// |-----------------|------------|--------------------------------------------------------|
/// | `Some(pfs)`     | `Some(lib)`| Workspace + library overlay                            |
/// | `Some(pfs)`     | `None`     | Workspace, no overlay                                  |
/// | `None`          | `*`        | Single-file (library overlay would require merging)    |
///
/// Note there is no `(None, Some(lib))` dedicated arm — the three live
/// variants are `file_*`, `workspace_*`, and `workspace_*_with_library`
/// (single-file with library overlay isn't a meaningful cache shape for
/// filter ExprIRs: filters reference user-authored types).
///
#[tracing::instrument(level = "debug", skip(db, project_files, library))]
#[salsa::tracked]
pub fn view_filter_exprs_best(
    db: &dyn Db,
    source_file: SourceFile,
    project_files: Option<ProjectFileSet>,
    library: Option<LibraryGraph>,
) -> CachedViewFilterExprs {
    match (project_files, library) {
        (Some(pfs), Some(lib)) => workspace_view_filter_exprs_with_library(db, pfs, lib),
        (Some(pfs), None) => workspace_view_filter_exprs(db, pfs),
        (None, _) => file_view_filter_exprs(db, source_file),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::AnalysisHost;

    #[test]
    fn file_filter_exprs_caches_across_calls() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            r#"package P { part def Engine; view def V { filter true; } }"#.to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let r1 = file_view_filter_exprs(analysis.db(), sf);
        let r2 = file_view_filter_exprs(analysis.db(), sf);

        assert!(Arc::ptr_eq(&r1.0, &r2.0));
    }

    #[test]
    fn file_filter_exprs_invalidates_on_content_change() {
        // Cache invalidation: any content change that produces a new
        // graph revision must produce a fresh Arc. Use a name-level
        // delta (mirrors the view_index test) so the salsa revision
        // bumps reliably regardless of whether the filter syntax
        // round-trips through every parse path.
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            r#"package P { view def V1; }"#.to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");
        let r1 = file_view_filter_exprs(host.analysis().db(), sf).arc();

        host.set_file_content(
            "test.sysml",
            r#"package P { view def V2; }"#.to_string(),
        );
        let sf2 = host.source_file(id).expect("source file still exists");
        let r2 = file_view_filter_exprs(host.analysis().db(), sf2).arc();

        assert!(!Arc::ptr_eq(&r1, &r2));
    }
}
