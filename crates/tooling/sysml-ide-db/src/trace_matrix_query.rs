//! Tracked queries for `trace_matrix` projection.
//!
//! Pre-S2.T17 (5/N) every `service.trace_matrix(uri, src_kind,
//! rel_kind, tgt_kind)` call walked every relationship of the given
//! kind, looked up source and target elements, checked subtype
//! membership, and built fresh `TraceMatrixRow` clones for the wire
//! response. The walk is O(relationships of `rel_kind`) and gets
//! called per requirements-coverage refresh, per verification-
//! status pass, per AI-agent traceability lookup. Pre-T17 the
//! result was discarded every time; post-T17 it's salsa-cached per
//! `(input, library, parse, source_kind, rel_kind, target_kind)`
//! tuple.
//!
//! Three variants mirror `eval_context` / `element_index` /
//! `descendants_query`:
//!
//! - `file_trace_matrix(db, sf, src, rel, tgt)` — single-file mode.
//! - `workspace_trace_matrix(db, pfs, src, rel, tgt)` — workspace
//!   (no library).
//! - `workspace_trace_matrix_with_library(db, pfs, lib, src, rel,
//!   tgt)` — workspace + library overlay.
//!
//! Result type `CachedTraceMatrix` wraps `Arc<Vec<TraceMatrixRow>>`
//! with pointer-identity equality (via `salsa_arc_wrapper!(identity,
//! …)`); salsa returns the same Arc on cache hits.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_core::{query, query::TraceMatrixRow, ElementKind, ModelGraph, RelationshipKind};

use crate::analysis::{elaborate_workspace, elaborate_workspace_with_library};
use crate::parse;
use crate::project_inputs::ProjectFileSet;
use crate::resolution::LibraryGraph;
use crate::source::SourceFile;
use crate::Db;

/// Salsa-cached `Vec<TraceMatrixRow>` snapshot.
///
/// Wraps `Arc<Vec<TraceMatrixRow>>` with pointer-identity equality
/// so salsa can memoize the value across queries even though
/// `TraceMatrixRow` itself isn't `Eq` (embeds `Option<String>`).
#[derive(Clone, Debug)]
pub struct CachedTraceMatrix(Arc<Vec<TraceMatrixRow>>);

impl CachedTraceMatrix {
    fn new(rows: Vec<TraceMatrixRow>) -> Self {
        Self(Arc::new(rows))
    }

    /// Borrow the inner row list.
    pub fn rows(&self) -> &[TraceMatrixRow] {
        &self.0
    }

    /// Clone the inner `Arc<Vec<TraceMatrixRow>>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<Vec<TraceMatrixRow>> {
        Arc::clone(&self.0)
    }

    /// Clone the inner `Vec<TraceMatrixRow>` (deep clone — only when
    /// callers need ownership).
    pub fn to_vec(&self) -> Vec<TraceMatrixRow> {
        (*self.0).clone()
    }
}

salsa_arc_wrapper!(identity, CachedTraceMatrix, Vec<TraceMatrixRow>);

fn build_trace_matrix(
    graph: &ModelGraph,
    source_kind: &ElementKind,
    rel_kind: &RelationshipKind,
    target_kind: &ElementKind,
) -> Vec<TraceMatrixRow> {
    query::trace_matrix(graph, source_kind, rel_kind, target_kind)
}

/// Build a trace matrix for a single-file model graph.
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_trace_matrix(
    db: &dyn Db,
    sf: SourceFile,
    source_kind: ElementKind,
    rel_kind: RelationshipKind,
    target_kind: ElementKind,
) -> CachedTraceMatrix {
    let parsed = parse::parse_file(db, sf);
    CachedTraceMatrix::new(build_trace_matrix(
        parsed.graph(),
        &source_kind,
        &rel_kind,
        &target_kind,
    ))
}

/// Build a trace matrix for the workspace-merged graph (no library).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_trace_matrix(
    db: &dyn Db,
    pfs: ProjectFileSet,
    source_kind: ElementKind,
    rel_kind: RelationshipKind,
    target_kind: ElementKind,
) -> CachedTraceMatrix {
    let elaborated = elaborate_workspace(db, pfs);
    CachedTraceMatrix::new(build_trace_matrix(
        elaborated.graph(),
        &source_kind,
        &rel_kind,
        &target_kind,
    ))
}

/// Build a trace matrix for the workspace-merged graph with the
/// standard library merged in.
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn workspace_trace_matrix_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
    source_kind: ElementKind,
    rel_kind: RelationshipKind,
    target_kind: ElementKind,
) -> CachedTraceMatrix {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    CachedTraceMatrix::new(build_trace_matrix(
        elaborated.graph(),
        &source_kind,
        &rel_kind,
        &target_kind,
    ))
}

/// Best-shape dispatcher — Some(lib) routes to ..._with_library, None to bare workspace.
// TODO(post-collapse): inline dispatch-parity tests
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_trace_matrix_best(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: Option<LibraryGraph>,
    source_kind: ElementKind,
    rel_kind: RelationshipKind,
    target_kind: ElementKind,
) -> CachedTraceMatrix {
    match library {
        Some(lib) => {
            workspace_trace_matrix_with_library(db, pfs, lib, source_kind, rel_kind, target_kind)
        }
        None => workspace_trace_matrix(db, pfs, source_kind, rel_kind, target_kind),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::AnalysisHost;

    const SAT_MODEL: &str = r#"
        package Demo {
            requirement def R1;
            part def P1 {
                satisfy R1;
            }
        }
    "#;

    #[test]
    fn file_trace_matrix_caches_across_calls() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content("test.sysml", SAT_MODEL.to_string());
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let m1 = file_trace_matrix(
            analysis.db(),
            sf,
            ElementKind::PartUsage,
            RelationshipKind::Satisfy,
            ElementKind::RequirementUsage,
        );
        let m2 = file_trace_matrix(
            analysis.db(),
            sf,
            ElementKind::PartUsage,
            RelationshipKind::Satisfy,
            ElementKind::RequirementUsage,
        );

        assert!(Arc::ptr_eq(&m1.0, &m2.0));
    }

    #[test]
    fn file_trace_matrix_invalidates_on_content_change() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content("test.sysml", SAT_MODEL.to_string());
        let sf = host.source_file(id).expect("source file exists");
        let m1 = file_trace_matrix(
            host.analysis().db(),
            sf,
            ElementKind::PartUsage,
            RelationshipKind::Satisfy,
            ElementKind::RequirementUsage,
        )
        .arc();

        host.set_file_content("test.sysml", "package Empty {}".to_string());
        let sf2 = host.source_file(id).expect("source file still exists");
        let m2 = file_trace_matrix(
            host.analysis().db(),
            sf2,
            ElementKind::PartUsage,
            RelationshipKind::Satisfy,
            ElementKind::RequirementUsage,
        )
        .arc();

        assert!(!Arc::ptr_eq(&m1, &m2));
    }

    #[test]
    fn file_trace_matrix_distinguishes_kinds() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content("test.sysml", SAT_MODEL.to_string());
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let satisfy = file_trace_matrix(
            analysis.db(),
            sf,
            ElementKind::PartUsage,
            RelationshipKind::Satisfy,
            ElementKind::RequirementUsage,
        );
        let verify = file_trace_matrix(
            analysis.db(),
            sf,
            ElementKind::PartUsage,
            RelationshipKind::Verify,
            ElementKind::RequirementUsage,
        );

        // Different relationship kind → distinct salsa slots → distinct Arcs.
        assert!(!Arc::ptr_eq(&satisfy.0, &verify.0));
    }
}
