//! Tracked queries for the snapshot-scoped `RefResolveCache`.
//!
//! `Orchestrator` populates a lazy compile-result cache during simulation:
//! every per-tick `Value::Ref(id)` resolution in `capture_snapshot` walks
//! the referenced element's expression subtree, compiles it to `ExprIR`,
//! and caches the result keyed by `ElementId`. Without that cache, every
//! resolvable attribute pays the AST walk + parse cost on every tick.
//!
//! Until this commit the cache was a private orchestrator field
//! (`Orchestrator.ref_resolve_cache: HashMap<ElementId, Option<Arc<ExprIR>>>`),
//! initialised empty per `Orchestrator::new` and `clear()`-ed on `reset()`.
//! That lifetime is **per session**: a `simulate.start` / fork / restart
//! drops the populated cache and the next run re-pays the compile cost.
//!
//! Per ADR-011 §6 (Q-RT-7), the cache is snapshot-scoped: it lives next
//! to the elaborated snapshot in `sysml-ide-db`, sharing the same
//! revision lifetime as the rest of the salsa-cached upstream bundle.
//! Each elaborated-graph revision returns one `Arc<Mutex<RefResolveCache>>`;
//! every orchestrator built against that revision shares the same Arc.
//! When the elaborated graph changes (file edit), salsa returns a new
//! Arc to a freshly-empty cache. The per-session `reset()` clear is no
//! longer needed because the cache lifetime tracks revision identity,
//! not session identity.
//!
//! ## Why mutability is OK to live inside a salsa-tracked value
//!
//! `RefResolveCache` is a `HashMap<ElementId, Option<Arc<ExprIR>>>`. Its
//! identity (the inner Arc) is what salsa memoizes; the salsa cache key
//! is the elaborated graph revision. Mutating the inner `HashMap`
//! doesn't change the salsa key — it only fills in compile results that
//! are pure functions of `(graph_rev, element_id)`. Different
//! orchestrators sharing the same Arc compute the same answer on the
//! same key, so sharing the populated cache is a *speedup*, not a
//! correctness hazard.
//!
//! ## Three variants mirror `eval_context` / `precompiled_constraints` /
//! `port_flow_runtime` / `gated_expressions` / `signal_expr_table`:
//!
//! - `file_ref_resolve_cache(db, source_file)` — single-file scope.
//! - `workspace_ref_resolve_cache(db, pfs)` — workspace-merged (no
//!   library overlay).
//! - `workspace_ref_resolve_cache_with_library(db, pfs, lib)` —
//!   workspace-merged with library overlay (the default for IDE / sim-app).
//!
//! Each variant forces a salsa dependency on its upstream parse /
//! elaborate query so any change to the elaborated graph invalidates
//! the cache (returning a new empty Arc on the next call).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use sysml_core::ElementId;
use sysml_runtime::expressions::ExprIR;

use crate::analysis::{elaborate_workspace, elaborate_workspace_with_library};
use crate::parse;
use crate::project_inputs::ProjectFileSet;
use crate::resolution::LibraryGraph;
use crate::source::SourceFile;
use crate::Db;

/// Inner cache type (mirrors `sysml_runtime::expressions::RefResolveCache`
/// but re-stated here so this module's surface doesn't drag a runtime
/// type alias into the ide-db public API — the cache is *shared with*
/// runtime, but ide-db owns its lifetime).
pub type RefResolveCacheMap = HashMap<ElementId, Option<Arc<ExprIR>>>;

/// Salsa-cached snapshot-scoped `RefResolveCache`.
///
/// Wraps `Arc<Mutex<RefResolveCacheMap>>` with pointer-identity equality
/// so salsa can memoize the value even though the inner type is not
/// `Eq`. Identity equality is the right semantics: the cache is a
/// *handle*, and two distinct Arcs are distinct handles even if their
/// contents are identical.
#[derive(Clone, Debug)]
pub struct CachedRefResolveCache(Arc<Mutex<RefResolveCacheMap>>);

impl CachedRefResolveCache {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(RefResolveCacheMap::new())))
    }

    /// Clone the inner `Arc<Mutex<RefResolveCacheMap>>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<Mutex<RefResolveCacheMap>> {
        Arc::clone(&self.0)
    }
}

salsa_arc_wrapper!(identity, CachedRefResolveCache, Mutex<RefResolveCacheMap>);

/// Empty snapshot-scoped cache for a single-file model graph.
///
/// Depends on `parse_file()` — any content change invalidates the cache
/// (returning a new empty `Arc` on the next call).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_ref_resolve_cache(
    db: &dyn Db,
    source_file: SourceFile,
) -> CachedRefResolveCache {
    // Force the salsa dependency on parse so file-content edits
    // invalidate this query and we hand out a fresh empty cache.
    let _parsed = parse::parse_file(db, source_file);
    CachedRefResolveCache::new()
}

/// Empty snapshot-scoped cache for a workspace-merged graph (no library overlay).
///
/// Depends on `elaborate_workspace()` — any change to the elaborated
/// workspace graph invalidates the cache.
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_ref_resolve_cache(
    db: &dyn Db,
    pfs: ProjectFileSet,
) -> CachedRefResolveCache {
    let _elaborated = elaborate_workspace(db, pfs);
    CachedRefResolveCache::new()
}

/// Empty snapshot-scoped cache for a workspace-merged graph with library overlay.
///
/// Depends on `elaborate_workspace_with_library()` — same invalidation
/// shape as the no-library variant, plus library churn.
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn workspace_ref_resolve_cache_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
) -> CachedRefResolveCache {
    let _elaborated = elaborate_workspace_with_library(db, pfs, library);
    CachedRefResolveCache::new()
}

/// Best-shape dispatcher — Some(lib) routes to ..._with_library, None to bare workspace.
// TODO(post-collapse): inline dispatch-parity tests
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_ref_resolve_cache_best(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: Option<LibraryGraph>,
) -> CachedRefResolveCache {
    match library {
        Some(lib) => workspace_ref_resolve_cache_with_library(db, pfs, lib),
        None => workspace_ref_resolve_cache(db, pfs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::AnalysisHost;

    #[test]
    fn file_ref_resolve_cache_caches_across_calls() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def Tank { attribute level : Real; } }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let r1 = file_ref_resolve_cache(analysis.db(), sf);
        let r2 = file_ref_resolve_cache(analysis.db(), sf);

        // Salsa returns the same Arc on cache hits — same handle, same
        // populated state for any concurrent orchestrator.
        assert!(Arc::ptr_eq(&r1.0, &r2.0));
    }

    #[test]
    fn file_ref_resolve_cache_invalidates_on_content_change() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package Foo { part def A { attribute x : Real; } }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");
        let r1 = file_ref_resolve_cache(host.analysis().db(), sf).arc();

        host.set_file_content(
            "test.sysml",
            "package Bar { part def B { attribute y : Real; } }".to_string(),
        );
        let sf2 = host.source_file(id).expect("source file still exists");
        let r2 = file_ref_resolve_cache(host.analysis().db(), sf2).arc();

        // Different content → different Arc to a fresh empty cache. Any
        // stale compile results from the old revision are dropped along
        // with the old Arc.
        assert!(!Arc::ptr_eq(&r1, &r2));
    }
}
