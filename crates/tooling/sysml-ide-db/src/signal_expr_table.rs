//! Tracked queries for the signal-expression table consumed by
//! `RuntimeSession::set_signals`.
//!
//! Today the service builds the table inline in `lib.rs` (the
//! `simulate.continuous.auto` branch): construct a `Snapshot` (which
//! builds a `ModelCompiler`), call `snap.detect_ode()` to get the first
//! `OdeDetection`, then iterate its `signal_exprs` HashMap and parse
//! each RHS to `ExprIR` via `ode_builder::parse_derivative`. Both the
//! ODE detection walk and the per-entry parse are pure graph
//! derivatives — the natural tracked-query target per ADR-011 §3 row
//! `cached_signal_expr_table` (RT-35).
//!
//! Three variants mirror `eval_context`, `precompiled_constraints`,
//! `port_flow_runtime`, and `gated_expressions`:
//!
//! - `file_signal_expr_table(db, source_file)` — single-file (no
//!   workspace, no library overlay).
//! - `workspace_signal_expr_table(db, pfs)` — workspace-merged (no
//!   library overlay).
//! - `workspace_signal_expr_table_with_library(db, pfs, lib)` —
//!   workspace-merged with library overlay (the default for IDE / sim-app).
//!
//! All three call `sysml_runtime::compiler::build_signal_expr_table` on
//! the elaborated graph.
//!
//! Note on first-call cost: `build_signal_expr_table` constructs a
//! `ModelCompiler` via `ModelCompiler::from_arc`, which today still
//! deep-clones the graph and re-elaborates it defensively before
//! running `detect_all_odes_unified` over the elaborated view. A
//! future ADR-011 step (routing through `Snapshot::compile_*`) lifts
//! that defensive re-elaborate; until then the first-call cost is one
//! clone + elaborate + walk. The salsa memo means every subsequent
//! call against an unchanged graph revision is a pure cache hit
//! regardless.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_runtime::compiler::build_signal_expr_table;
use sysml_runtime::expressions::ExprIR;

use crate::analysis::{elaborate_workspace, elaborate_workspace_with_library};
use crate::parse;
use crate::project_inputs::ProjectFileSet;
use crate::resolution::LibraryGraph;
use crate::source::SourceFile;
use crate::Db;

/// Salsa-cached signal-expression table.
///
/// Wraps `Arc<Vec<(String, ExprIR)>>` with pointer-identity equality so
/// salsa can memoize the value even though the inner `ExprIR` does not
/// implement `Eq`.
#[derive(Clone, Debug)]
pub struct CachedSignalExprTable(Arc<Vec<(String, ExprIR)>>);

impl CachedSignalExprTable {
    fn new(entries: Vec<(String, ExprIR)>) -> Self {
        Self(Arc::new(entries))
    }

    /// Borrow the inner `Vec<(String, ExprIR)>`.
    pub fn entries(&self) -> &[(String, ExprIR)] {
        &self.0
    }

    /// Clone the inner `Arc<Vec<(String, ExprIR)>>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<Vec<(String, ExprIR)>> {
        Arc::clone(&self.0)
    }
}

salsa_arc_wrapper!(identity, CachedSignalExprTable, Vec<(String, ExprIR)>);

/// Build the signal-expression table for a single-file model graph.
///
/// Depends on: `parse_file()` (Layer 1).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_signal_expr_table(
    db: &dyn Db,
    source_file: SourceFile,
) -> CachedSignalExprTable {
    let parsed = parse::parse_file(db, source_file);
    let entries = build_signal_expr_table(parsed.graph());
    CachedSignalExprTable::new(entries)
}

/// Build the signal-expression table for a workspace-merged graph (no library overlay).
///
/// Depends on: `elaborate_workspace()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_signal_expr_table(
    db: &dyn Db,
    pfs: ProjectFileSet,
) -> CachedSignalExprTable {
    let elaborated = elaborate_workspace(db, pfs);
    let entries = build_signal_expr_table(elaborated.graph());
    CachedSignalExprTable::new(entries)
}

/// Build the signal-expression table for a workspace-merged graph with library overlay.
///
/// Depends on: `elaborate_workspace_with_library()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn workspace_signal_expr_table_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
) -> CachedSignalExprTable {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    let entries = build_signal_expr_table(elaborated.graph());
    CachedSignalExprTable::new(entries)
}

/// Best-shape dispatcher — Some(lib) routes to ..._with_library, None to bare workspace.
// TODO(post-collapse): inline dispatch-parity tests
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_signal_expr_table_best(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: Option<LibraryGraph>,
) -> CachedSignalExprTable {
    match library {
        Some(lib) => workspace_signal_expr_table_with_library(db, pfs, lib),
        None => workspace_signal_expr_table(db, pfs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::AnalysisHost;

    #[test]
    fn file_signal_expr_table_caches_across_calls() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def Tank { attribute level : Real; } }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let r1 = file_signal_expr_table(analysis.db(), sf);
        let r2 = file_signal_expr_table(analysis.db(), sf);

        // Salsa returns the same Arc on cache hits.
        assert!(Arc::ptr_eq(&r1.0, &r2.0));
    }

    #[test]
    fn file_signal_expr_table_invalidates_on_content_change() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package Foo { part def A { attribute x : Real; } }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");
        let r1 = file_signal_expr_table(host.analysis().db(), sf).arc();

        host.set_file_content(
            "test.sysml",
            "package Bar { part def B { attribute y : Real; } }".to_string(),
        );
        let sf2 = host.source_file(id).expect("source file still exists");
        let r2 = file_signal_expr_table(host.analysis().db(), sf2).arc();

        // Different content → different Arc.
        assert!(!Arc::ptr_eq(&r1, &r2));
    }
}
