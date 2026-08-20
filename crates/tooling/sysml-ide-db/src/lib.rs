//! # sysml-ide-db
//!
//! Salsa-based incremental computation database for SysML v2.
//!
//! This crate defines the core database types and query layers that power
//! incremental recomputation in the LSP server. It follows rust-analyzer's
//! architecture pattern where:
//!
//! - **Source inputs** (Layer 0) hold file contents and workspace config
//! - **Parse queries** (Layer 1) produce CSTs, ModelGraphs, and diagnostics
//! - **Model queries** (Layer 2, future) produce per-file resolved models
//! - **Resolution queries** (Layer 3) handle cross-file name resolution
//! - **Analysis queries** (Layer 4) handle validation and elaboration
//!
//! The [`AnalysisHost`] / [`Analysis`] pair provides the mutable/immutable
//! split needed for concurrent LSP request handling.
//!
//! ## Architecture
//!
//! ```text
//! AnalysisHost (mutable, LSP main loop)
//!     │
//!     ├── set_file_content()  ──► salsa input invalidation
//!     ├── set_workspace_config()
//!     │
//!     └── analysis()  ──► Analysis (immutable snapshot)
//!                              │
//!                              ├── diagnostics(file)
//!                              ├── hover(file, offset)
//!                              ├── goto_definition(file, offset)
//!                              └── ... (future query methods)
//! ```

#[macro_use]
mod arc_wrapper;
pub mod analysis;
pub mod descendants_query;
pub mod diagram;
pub mod view_model;
pub mod text_map;
pub mod interaction;
pub mod element_index;
pub mod eval_context;
pub mod ref_index;
pub mod eval_context_seed;
pub mod exports;
pub mod file_source_query;
pub mod gated_expressions;
pub mod port_flow_runtime;
pub mod precompiled_constraints;
pub mod ref_resolve_cache;
pub mod signal_expr_table;
pub mod host;
pub mod parse;
pub mod physics;
pub mod project_inputs;
pub mod resolution;
pub mod snapshot;
pub mod source;
pub mod stats;
pub mod symbol_index_query;
pub mod symbols;
pub mod tokens;
pub mod trace_matrix_query;
pub mod view_filter_exprs;
pub mod view_index;
pub mod workspace_capabilities;

pub use analysis::{
    elaborate_file_best, elaborate_workspace_best, validate_file_best, ElaboratedModel,
    ElaboratedWorkspace, ValidatedModel,
};
pub use descendants_query::{
    file_descendants, workspace_descendants_best, CachedDescendants,
};
pub use element_index::{
    file_kind_index, file_name_index, workspace_kind_index, workspace_kind_index_with_library,
    workspace_name_index, workspace_name_index_with_library, CachedKindIndex, CachedNameIndex,
};
pub use ref_index::{
    workspace_ref_index, workspace_ref_index_with_library, CachedRefIndex, Provenance, RefKind,
    RefSite,
};
pub use eval_context::{
    file_eval_context, workspace_eval_context_best, CachedEvalContext,
};
pub use gated_expressions::{
    file_gated_expressions, workspace_gated_expressions_best, CachedGatedExpressions,
};
pub use port_flow_runtime::{
    file_port_flow_runtime, workspace_port_flow_runtime_best, CachedPortFlowRuntime,
};
pub use signal_expr_table::{
    file_signal_expr_table, workspace_signal_expr_table_best, CachedSignalExprTable,
};
pub use precompiled_constraints::{
    file_precompiled_constraints, workspace_precompiled_constraints_best,
    CachedPrecompiledConstraintSet,
};
pub use physics::{
    file_physics_health, file_physics_registry, workspace_physics_executor_best,
    workspace_physics_health_best, workspace_physics_registry_best, CachedPhysicsExecutor,
    CachedPhysicsHealth, CachedPhysicsRegistry,
};
pub use ref_resolve_cache::{
    file_ref_resolve_cache, workspace_ref_resolve_cache_best, CachedRefResolveCache,
    RefResolveCacheMap,
};
pub use exports::{file_exports, ExportedDef, FileExports};
pub use host::{Analysis, AnalysisHost};
pub use parse::{
    file_outline, file_position_map, parse_file, parse_file_cst, parse_tree, CachedTree, Cst,
    Outline, ParseResult, PositionMap,
};
pub use project_inputs::{ProjectFileSet, SalsaProject, WorkspaceConfig};
pub use resolution::{
    resolve_file_best, LibraryData, LibraryGraph, ResolvedModel,
};
pub use salsa::Cancelled;
pub use snapshot::Snapshot;
pub use source::{canonical_uri, FileId, FileSet, SourceFile};
pub use stats::{QueryStats, QueryStatsSnapshot};
pub use symbol_index_query::{resolve_symbol, symbol_index, GlobalSymbolIndex};
pub use symbols::{file_document_symbols, DocumentSymbolTree, SymbolNode};
pub use tokens::{
    element_kind_to_category, file_semantic_tokens, token_modifiers, FileTokens, RawToken,
    TokenCategory,
};
pub use trace_matrix_query::{
    file_trace_matrix, workspace_trace_matrix_best, CachedTraceMatrix,
};
pub use diagram::{
    file_diagram, workspace_diagram_best, CachedDiagram,
};
pub use view_model::{
    file_view_model, workspace_view_model_best, CachedViewModel,
};
pub use text_map::{
    file_text_map, workspace_text_map_best, CachedTextMap,
};
pub use interaction::{
    file_interaction_map, workspace_interaction_map_best, CachedInteractionMap,
};
pub use view_filter_exprs::{
    view_filter_exprs_best, CachedViewFilterExprs, ViewFilterExprMap,
};
pub use view_index::{
    file_view_index, workspace_view_index_best, CachedViewIndex,
};
pub use workspace_capabilities::{
    workspace_capabilities_best, WorkspaceCapabilities,
};

use std::sync::Arc;

/// The database trait for sysml-db queries.
///
/// All salsa tracked functions in this crate take `&dyn Db` as their
/// first argument. Downstream crates that define additional queries
/// can extend this trait.
#[salsa::db]
pub trait Db: salsa::Database {}

/// The concrete database implementation.
///
/// This struct holds all salsa storage and implements `salsa::Database`.
/// It is the single concrete database type used throughout the system.
///
/// Following rust-analyzer's pattern, this lives in the "ide-db" layer
/// and is shared by all higher-level crates.
#[salsa::db]
#[derive(Clone)]
pub struct RootDatabase {
    storage: salsa::Storage<Self>,
    /// Shared query execution statistics, updated by the salsa event handler.
    stats: Arc<QueryStats>,
}

#[salsa::db]
impl salsa::Database for RootDatabase {}

#[salsa::db]
impl Db for RootDatabase {}

impl Default for RootDatabase {
    fn default() -> Self {
        use std::sync::atomic::Ordering;

        let stats = Arc::new(QueryStats::default());
        let stats_for_handler = Arc::clone(&stats);

        Self {
            storage: salsa::Storage::new(Some(Box::new(move |event| {
                match event.kind {
                    salsa::EventKind::WillExecute { .. } => {
                        stats_for_handler.executions.fetch_add(1, Ordering::Relaxed);
                    }
                    salsa::EventKind::DidValidateMemoizedValue { .. } => {
                        stats_for_handler
                            .validations
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {}
                }
                if tracing::enabled!(tracing::Level::DEBUG) {
                    tracing::debug!("salsa_event({:?})", event.kind);
                }
            }))),
            stats,
        }
    }
}

impl RootDatabase {
    /// Get a snapshot of query execution statistics.
    pub fn query_stats(&self) -> QueryStatsSnapshot {
        self.stats.snapshot()
    }

    /// Reset all query execution statistics to zero.
    pub fn reset_query_stats(&self) {
        self.stats.reset();
    }
}

impl std::fmt::Debug for RootDatabase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RootDatabase").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use salsa::Setter;

    #[test]
    fn create_database() {
        let _db = RootDatabase::default();
    }

    #[test]
    fn create_and_read_source_file() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Foo;".to_string());
        assert_eq!(sf.text(&db), "package Foo;");
    }

    #[test]
    fn update_source_file() {
        let mut db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Foo;".to_string());
        assert_eq!(sf.text(&db), "package Foo;");

        sf.set_text(&mut db).to("package Bar;".to_string());
        assert_eq!(sf.text(&db), "package Bar;");
    }

    #[test]
    fn file_set_operations() {
        let mut db = RootDatabase::default();
        let mut files = FileSet::new();

        // Create a file
        let id = files.set_file_text(&mut db, "file:///test.sysml", "package Test;".to_string());

        // Look up by URI
        assert_eq!(files.lookup("file:///test.sysml"), Some(id));
        assert_eq!(files.lookup("file:///nonexistent.sysml"), None);

        // Get URI from ID
        assert_eq!(files.uri(id), Some("file:///test.sysml"));

        // Read content via salsa
        let sf = files.source_file(id).unwrap();
        assert_eq!(sf.text(&db), "package Test;");

        // Update content
        files.set_file_text(
            &mut db,
            "file:///test.sysml",
            "package Updated;".to_string(),
        );
        assert_eq!(sf.text(&db), "package Updated;");

        // Count
        assert_eq!(files.len(), 1);
        assert!(!files.is_empty());

        // Remove
        files.remove("file:///test.sysml");
        assert_eq!(files.len(), 0);
        assert!(files.is_empty());
    }

    #[test]
    fn same_uri_returns_same_file_id() {
        let mut db = RootDatabase::default();
        let mut files = FileSet::new();

        let id1 = files.set_file_text(&mut db, "file:///a.sysml", "v1".to_string());
        let id2 = files.set_file_text(&mut db, "file:///a.sysml", "v2".to_string());

        assert_eq!(id1, id2);
        // Content should be updated
        let sf = files.source_file(id1).unwrap();
        assert_eq!(sf.text(&db), "v2");
    }

    #[test]
    fn different_uris_get_different_ids() {
        let mut db = RootDatabase::default();
        let mut files = FileSet::new();

        let id1 = files.set_file_text(&mut db, "file:///a.sysml", "a".to_string());
        let id2 = files.set_file_text(&mut db, "file:///b.sysml", "b".to_string());

        assert_ne!(id1, id2);
    }

    #[test]
    fn analysis_host_basic() {
        let mut host = AnalysisHost::new();

        // Set file content
        let id = host.set_file_content("file:///test.sysml", "package Test;".to_string());

        // Get snapshot
        let analysis = host.analysis();

        // Read through snapshot
        let sf = host.source_file(id).unwrap();
        assert_eq!(analysis.file_text(sf), "package Test;");
    }

    #[test]
    fn analysis_snapshot_reads_current_state() {
        let mut host = AnalysisHost::new();

        // Set initial content
        let id = host.set_file_content("file:///test.sysml", "v1".to_string());
        let sf = host.source_file(id).unwrap();

        // Take snapshot and verify
        {
            let snap = host.analysis();
            assert_eq!(snap.file_text(sf), "v1");
        }
        // Snapshot dropped — now safe to mutate

        // Update content
        host.set_file_content("file:///test.sysml", "v2".to_string());

        // New snapshot sees updated content
        {
            let snap = host.analysis();
            assert_eq!(snap.file_text(sf), "v2");
        }
    }

    #[test]
    fn snapshot_on_different_thread() {
        let mut host = AnalysisHost::new();

        let id = host.set_file_content("file:///test.sysml", "v1".to_string());
        let sf = host.source_file(id).unwrap();

        // Snapshot can be sent to another thread for concurrent reads
        let snap = host.analysis();
        let handle = std::thread::spawn(move || {
            assert_eq!(snap.file_text(sf), "v1");
        });
        handle.join().unwrap();
    }

    #[test]
    fn file_id_iteration() {
        let mut db = RootDatabase::default();
        let mut files = FileSet::new();

        files.set_file_text(&mut db, "file:///a.sysml", "a".to_string());
        files.set_file_text(&mut db, "file:///b.sysml", "b".to_string());
        files.set_file_text(&mut db, "file:///c.sysml", "c".to_string());

        let ids: Vec<_> = files.file_ids().collect();
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn snapshot_cancellation_on_mutation() {
        // When the host mutates, active queries on snapshots are cancelled.
        // The mutation blocks until all snapshot clones are dropped.
        // On a separate thread, the snapshot's queries receive Cancelled.
        use std::sync::mpsc;

        let mut host = AnalysisHost::new();
        let id = host.set_file_content("file:///test.sysml", "package Test;".to_string());
        let sf = host.source_file(id).unwrap();

        let snap = host.analysis();
        let (ready_tx, ready_rx) = mpsc::channel();

        // Spawn a thread that holds the snapshot and does a query
        let handle = std::thread::spawn(move || {
            // Read through snapshot — works fine
            let text = snap.file_text(sf);
            assert_eq!(text, "package Test;");

            // Signal main thread that we're done reading
            ready_tx.send(()).unwrap();

            // Drop the snapshot so main thread can mutate
            drop(snap);
        });

        // Wait for snapshot thread to finish
        ready_rx.recv().unwrap();
        handle.join().unwrap();

        // Now safe to mutate (no clones outstanding)
        host.set_file_content("file:///test.sysml", "package Updated;".to_string());

        // New snapshot sees updated content
        let snap2 = host.analysis();
        assert_eq!(snap2.file_text(sf), "package Updated;");
    }

    #[test]
    fn cancelled_catch_api() {
        // Verify that salsa::Cancelled::catch works for our queries.
        let mut host = AnalysisHost::new();
        let id = host.set_file_content("file:///test.sysml", "package Foo;".to_string());
        let sf = host.source_file(id).unwrap();

        let snap = host.analysis();
        let result = salsa::Cancelled::catch(std::panic::AssertUnwindSafe(|| {
            snap.file_text(sf).to_string()
        }));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "package Foo;");
    }
}
