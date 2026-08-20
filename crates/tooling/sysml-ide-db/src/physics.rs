//! Tracked queries for the physics static-analysis layer.
//!
//! Wraps the two graph-derived entry points in `sysml-core::physics`:
//!
//! - `PhysicsDomainRegistry::from_workspace_graph(graph)` — walks every
//!   `AttributeDefinition` that specializes `ScalarQuantityValue`,
//!   finds its associated unit element, and extracts a dimension
//!   vector. Pure function of the elaborated graph.
//! - `physics_health_diagnostics(graph)` — runs PH001-PH006 checks
//!   (domain mismatches, direction conflicts, missing effort/flow
//!   pairs, conservation imbalances, unwired RCI elements, Real-typed
//!   physics attributes). Pure function of the elaborated graph; the
//!   registry it builds internally is rebuilt from the graph for every
//!   call, which the cache amortises.
//!
//! Caching closes ADR-011 §3 / S3.T11. The registry is reused by hover
//! and code-lens (`classify_port_definition` takes `&PhysicsDomainRegistry`)
//! so a populated cache turns each per-port classification into a graph
//! walk plus dimension-vector lookups instead of a workspace re-walk.
//!
//! Three variants per query mirror `signal_expr_table`,
//! `ref_resolve_cache`, `gated_expressions`, and the other tracked-query
//! modules in this crate:
//!
//! - `file_*` — single-file (no workspace, no library overlay).
//! - `workspace_*` — workspace-merged (no library overlay).
//! - `workspace_*_with_library` — workspace-merged with library overlay
//!   (the default for IDE / sim-app).

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_core::physics::PhysicsDomainRegistry;
use sysml_runtime::physics::PhysicsExecutor;
use sysml_span::Diagnostic;

use crate::analysis::{elaborate_workspace, elaborate_workspace_with_library};
use crate::parse;
use crate::project_inputs::ProjectFileSet;
use crate::resolution::LibraryGraph;
use crate::source::SourceFile;
use crate::Db;

/// Salsa-cached `PhysicsDomainRegistry` derived from the elaborated graph.
///
/// Wraps `Arc<PhysicsDomainRegistry>` with pointer-identity equality so
/// salsa can memoize the value even though `PhysicsDomainRegistry` is
/// large (carries a `HashMap<String, DimensionVector>` populated from
/// the workspace).
#[derive(Clone, Debug)]
pub struct CachedPhysicsRegistry(Arc<PhysicsDomainRegistry>);

impl CachedPhysicsRegistry {
    fn new(registry: PhysicsDomainRegistry) -> Self {
        Self(Arc::new(registry))
    }

    /// Borrow the inner `PhysicsDomainRegistry`.
    pub fn registry(&self) -> &PhysicsDomainRegistry {
        &self.0
    }

    /// Clone the inner `Arc<PhysicsDomainRegistry>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<PhysicsDomainRegistry> {
        Arc::clone(&self.0)
    }
}

salsa_arc_wrapper!(identity, CachedPhysicsRegistry, PhysicsDomainRegistry);

/// Salsa-cached output of `physics_health_diagnostics(graph)`.
///
/// Wraps `Arc<Vec<Diagnostic>>` with pointer-identity equality. The
/// inner `Diagnostic` does not implement `Eq`; identity equality is
/// sufficient because the diagnostics value is only compared within
/// the same salsa revision.
#[derive(Clone, Debug)]
pub struct CachedPhysicsHealth(Arc<Vec<Diagnostic>>);

impl CachedPhysicsHealth {
    fn new(diags: Vec<Diagnostic>) -> Self {
        Self(Arc::new(diags))
    }

    /// Borrow the inner diagnostics slice.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.0
    }

    /// Clone the inner `Arc<Vec<Diagnostic>>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<Vec<Diagnostic>> {
        Arc::clone(&self.0)
    }
}

salsa_arc_wrapper!(identity, CachedPhysicsHealth, Vec<Diagnostic>);

// ---------------------------------------------------------------------------
// PhysicsDomainRegistry
// ---------------------------------------------------------------------------

/// Build a `PhysicsDomainRegistry` for a single-file model graph.
///
/// Depends on: `parse_file()` (Layer 1).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_physics_registry(db: &dyn Db, source_file: SourceFile) -> CachedPhysicsRegistry {
    let parsed = parse::parse_file(db, source_file);
    let registry = PhysicsDomainRegistry::from_workspace_graph(parsed.graph());
    CachedPhysicsRegistry::new(registry)
}

/// Build a `PhysicsDomainRegistry` for a workspace-merged graph (no library overlay).
///
/// Depends on: `elaborate_workspace()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_physics_registry(db: &dyn Db, pfs: ProjectFileSet) -> CachedPhysicsRegistry {
    let elaborated = elaborate_workspace(db, pfs);
    let registry = PhysicsDomainRegistry::from_workspace_graph(elaborated.graph());
    CachedPhysicsRegistry::new(registry)
}

/// Build a `PhysicsDomainRegistry` for a workspace-merged graph with library overlay.
///
/// Depends on: `elaborate_workspace_with_library()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn workspace_physics_registry_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
) -> CachedPhysicsRegistry {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    let registry = PhysicsDomainRegistry::from_workspace_graph(elaborated.graph());
    CachedPhysicsRegistry::new(registry)
}

/// Best-shape dispatcher — Some(lib) routes to ..._with_library, None to bare workspace.
// TODO(post-collapse): inline dispatch-parity tests
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_physics_registry_best(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: Option<LibraryGraph>,
) -> CachedPhysicsRegistry {
    match library {
        Some(lib) => workspace_physics_registry_with_library(db, pfs, lib),
        None => workspace_physics_registry(db, pfs),
    }
}

// ---------------------------------------------------------------------------
// physics_health_diagnostics
// ---------------------------------------------------------------------------

/// Run physics health diagnostics on a single-file model graph.
///
/// Depends on: `parse_file()` (Layer 1).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_physics_health(db: &dyn Db, source_file: SourceFile) -> CachedPhysicsHealth {
    let parsed = parse::parse_file(db, source_file);
    let diags = sysml_core::physics::health::physics_health_diagnostics(parsed.graph());
    CachedPhysicsHealth::new(diags)
}

/// Run physics health diagnostics on a workspace-merged graph (no library overlay).
///
/// Depends on: `elaborate_workspace()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_physics_health(db: &dyn Db, pfs: ProjectFileSet) -> CachedPhysicsHealth {
    let elaborated = elaborate_workspace(db, pfs);
    let diags = sysml_core::physics::health::physics_health_diagnostics(elaborated.graph());
    CachedPhysicsHealth::new(diags)
}

/// Run physics health diagnostics on a workspace-merged graph with library overlay.
///
/// Depends on: `elaborate_workspace_with_library()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn workspace_physics_health_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
) -> CachedPhysicsHealth {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    let diags = sysml_core::physics::health::physics_health_diagnostics(elaborated.graph());
    CachedPhysicsHealth::new(diags)
}

/// Best-shape dispatcher — Some(lib) routes to ..._with_library, None to bare workspace.
// TODO(post-collapse): inline dispatch-parity tests
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_physics_health_best(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: Option<LibraryGraph>,
) -> CachedPhysicsHealth {
    match library {
        Some(lib) => workspace_physics_health_with_library(db, pfs, lib),
        None => workspace_physics_health(db, pfs),
    }
}

// ---------------------------------------------------------------------------
// PhysicsExecutor (RSC-6.4)
// ---------------------------------------------------------------------------

/// Salsa-cached `PhysicsExecutor` derived from the elaborated workspace graph.
///
/// Wraps `Option<Arc<PhysicsExecutor>>`: `None` when the model has no PowerBond
/// physics topology (`PhysicsExecutor::from_graph` returns `Err` — not an
/// error, just "no physics needed"). Pointer-identity equality on the inner
/// `Arc` lets salsa memoize the (large) executor; two `None`s compare equal.
///
/// The executor is built from `elaborate_workspace[_with_library]`'s graph —
/// the SAME elaborated graph the service feeds to `Snapshot` /
/// `build_workspace_orchestrator` — and `from_graph` classifies links via
/// `build_port_flow_resources().registry`, which is exactly the `compile_ports`
/// registry the orchestrator's inline step 7 uses. So a cached executor threaded
/// through `ModelCompiler::with_cached_physics_executor` is byte-identical to
/// the inline build (RSC-6.4). The build was previously redone on every
/// orchestrator construction; this query amortises it to once per graph version.
#[derive(Clone)]
pub struct CachedPhysicsExecutor(Option<Arc<PhysicsExecutor>>);

impl std::fmt::Debug for CachedPhysicsExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // PhysicsExecutor is not Debug; report presence/absence only.
        f.debug_tuple("CachedPhysicsExecutor")
            .field(&self.0.is_some())
            .finish()
    }
}

impl CachedPhysicsExecutor {
    fn new(executor: Option<PhysicsExecutor>) -> Self {
        Self(executor.map(Arc::new))
    }

    /// Clone the inner `Arc<PhysicsExecutor>` if a physics topology exists.
    pub fn arc(&self) -> Option<Arc<PhysicsExecutor>> {
        self.0.clone()
    }
}

impl PartialEq for CachedPhysicsExecutor {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}
impl Eq for CachedPhysicsExecutor {}
impl Hash for CachedPhysicsExecutor {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match &self.0 {
            Some(a) => Arc::as_ptr(a).hash(state),
            None => std::ptr::null::<PhysicsExecutor>().hash(state),
        }
    }
}

/// Build the workspace physics executor (no library overlay).
///
/// Depends on: `elaborate_workspace()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_physics_executor(db: &dyn Db, pfs: ProjectFileSet) -> CachedPhysicsExecutor {
    let elaborated = elaborate_workspace(db, pfs);
    let executor = PhysicsExecutor::from_graph(elaborated.graph())
        .ok()
        .map(|(exec, _diags)| exec);
    CachedPhysicsExecutor::new(executor)
}

/// Build the workspace physics executor with library overlay.
///
/// Depends on: `elaborate_workspace_with_library()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn workspace_physics_executor_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
) -> CachedPhysicsExecutor {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    let executor = PhysicsExecutor::from_graph(elaborated.graph())
        .ok()
        .map(|(exec, _diags)| exec);
    CachedPhysicsExecutor::new(executor)
}

/// Best-shape dispatcher — Some(lib) routes to ..._with_library, None to bare workspace.
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_physics_executor_best(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: Option<LibraryGraph>,
) -> CachedPhysicsExecutor {
    match library {
        Some(lib) => workspace_physics_executor_with_library(db, pfs, lib),
        None => workspace_physics_executor(db, pfs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::AnalysisHost;

    #[test]
    fn file_physics_registry_caches_across_calls() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def Tank { attribute level : Real; } }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let r1 = file_physics_registry(analysis.db(), sf);
        let r2 = file_physics_registry(analysis.db(), sf);

        assert!(Arc::ptr_eq(&r1.0, &r2.0));
    }

    #[test]
    fn file_physics_registry_invalidates_on_content_change() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package Foo { part def A { attribute x : Real; } }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");
        let r1 = file_physics_registry(host.analysis().db(), sf).arc();

        host.set_file_content(
            "test.sysml",
            "package Bar { part def B { attribute y : Real; } }".to_string(),
        );
        let sf2 = host.source_file(id).expect("source file still exists");
        let r2 = file_physics_registry(host.analysis().db(), sf2).arc();

        assert!(!Arc::ptr_eq(&r1, &r2));
    }

    #[test]
    fn file_physics_health_caches_across_calls() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def Tank { attribute level : Real; } }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let r1 = file_physics_health(analysis.db(), sf);
        let r2 = file_physics_health(analysis.db(), sf);

        assert!(Arc::ptr_eq(&r1.0, &r2.0));
    }

    #[test]
    fn file_physics_health_invalidates_on_content_change() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package Foo { part def A { attribute x : Real; } }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");
        let r1 = file_physics_health(host.analysis().db(), sf).arc();

        host.set_file_content(
            "test.sysml",
            "package Bar { part def B { attribute y : Real; } }".to_string(),
        );
        let sf2 = host.source_file(id).expect("source file still exists");
        let r2 = file_physics_health(host.analysis().db(), sf2).arc();

        assert!(!Arc::ptr_eq(&r1, &r2));
    }
}
