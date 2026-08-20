//! Tracked queries for `sysml_runtime::EvalContext` construction.
//!
//! `EvalContext` is built from a `ModelGraph` by walking every named element
//! and binding default values. The walk is O(n) and gets called per LSP
//! keystroke (inlay hints), per simulation start, per constraint check, and
//! per evaluate call. Pre-S2.T17 the service rebuilt it on every call —
//! these tracked queries collapse that to a single salsa-cached compute
//! per (input, library, parse) triple.
//!
//! Three variants:
//!
//! - `file_eval_context(db, source_file)` — single-file mode (no workspace,
//!   no library merge). Used when only one file is loaded.
//! - `workspace_eval_context(db, pfs)` — workspace-merged mode (no library).
//! - `workspace_eval_context_with_library(db, pfs, lib)` — workspace-merged
//!   with library overlay (default for IDE / sim-app).
//!
//! Result type `CachedEvalContext` wraps `Arc<EvalContext>` with
//! identity-based equality (via `salsa_arc_wrapper!(identity, …)`); salsa
//! returns the same Arc on cache hits, so downstream consumers benefit
//! transparently.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_runtime::expressions::EvalContext;

use crate::eval_context_seed::context_from_graph;

use crate::analysis::{elaborate_workspace, elaborate_workspace_with_library};
use crate::parse;
use crate::project_inputs::ProjectFileSet;
use crate::resolution::LibraryGraph;
use crate::source::SourceFile;
use crate::Db;

/// Salsa-cached `EvalContext` snapshot.
///
/// Wraps `Arc<EvalContext>` with pointer-identity equality so salsa can
/// memoize the value across queries even though `EvalContext` itself is
/// not `Eq` (it embeds `HashMap<String, Value>`, `Arc<ModelGraph>`, etc.).
#[derive(Clone, Debug)]
pub struct CachedEvalContext(Arc<EvalContext>);

impl CachedEvalContext {
    fn new(ctx: EvalContext) -> Self {
        Self(Arc::new(ctx))
    }

    /// Borrow the inner `EvalContext`.
    pub fn ctx(&self) -> &EvalContext {
        &self.0
    }

    /// Clone the inner `Arc<EvalContext>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<EvalContext> {
        Arc::clone(&self.0)
    }
}

salsa_arc_wrapper!(identity, CachedEvalContext, EvalContext);

/// Build an `EvalContext` for a single-file model graph.
///
/// Depends on: `parse_file()` (Layer 1).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_eval_context(db: &dyn Db, source_file: SourceFile) -> CachedEvalContext {
    let parsed = parse::parse_file(db, source_file);
    let graph_arc = Arc::new(parsed.graph().clone());
    let ctx = context_from_graph(&graph_arc);
    CachedEvalContext::new(ctx)
}

/// Build an `EvalContext` for a workspace-merged graph (no library overlay).
///
/// Depends on: `elaborate_workspace()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_eval_context(db: &dyn Db, pfs: ProjectFileSet) -> CachedEvalContext {
    let elaborated = elaborate_workspace(db, pfs);
    let ctx = context_from_graph(elaborated.graph());
    CachedEvalContext::new(ctx)
}

/// Build an `EvalContext` for a workspace-merged graph with library overlay.
///
/// Depends on: `elaborate_workspace_with_library()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn workspace_eval_context_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
) -> CachedEvalContext {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    let ctx = context_from_graph(elaborated.graph());
    CachedEvalContext::new(ctx)
}

/// Best-shape dispatcher — Some(lib) routes to ..._with_library, None to bare workspace.
// TODO(post-collapse): inline dispatch-parity tests
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_eval_context_best(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: Option<LibraryGraph>,
) -> CachedEvalContext {
    match library {
        Some(lib) => workspace_eval_context_with_library(db, pfs, lib),
        None => workspace_eval_context(db, pfs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::AnalysisHost;

    #[test]
    fn file_eval_context_caches_across_calls() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content("test.sysml", "package Foo {}".to_string());
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let ctx1 = file_eval_context(analysis.db(), sf);
        let ctx2 = file_eval_context(analysis.db(), sf);

        // Salsa returns the same Arc on cache hits.
        assert!(Arc::ptr_eq(&ctx1.0, &ctx2.0));
    }

    #[test]
    fn file_eval_context_invalidates_on_content_change() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content("test.sysml", "package Foo {}".to_string());
        let sf = host.source_file(id).expect("source file exists");
        let ctx1 = file_eval_context(host.analysis().db(), sf).arc();

        host.set_file_content("test.sysml", "package Bar {}".to_string());
        let sf2 = host.source_file(id).expect("source file still exists");
        let ctx2 = file_eval_context(host.analysis().db(), sf2).arc();

        // Different content → different Arc.
        assert!(!Arc::ptr_eq(&ctx1, &ctx2));
    }
}
