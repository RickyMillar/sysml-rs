//! Tracked queries for `sysml_runtime::flows::PortFlowResources`.
//!
//! The port+flow compile (`compile_ports` + `compile_flows`) walks every
//! `PortUsage`, `FlowUsage`, `SuccessionFlowUsage`, and KerML `Flow` in the
//! model graph, resolves elaborated direction / conjugation / definition
//! properties, and extracts typed features from PortDefinition children.
//! On stdlib-heavy workspaces (large multi-circuit models, ~hundreds of ports
//! across the live circuits) the walk costs a few milliseconds per call;
//! it was previously re-run on every `build_workspace_orchestrator`.
//!
//! Per ADR-011 §3 rows `cached_port_registry` (RT-17) +
//! `cached_flow_ir` (RT-18 connections + RT-19 gates), this is a pure
//! graph derivative and the natural tracked-query target. The two rows
//! are bundled into one cached value because:
//!
//! - both walks share the same graph-revision invalidation key,
//! - both feed the compiler's orchestrator-assembly path (the registry to
//!   `Orchestrator::set_port_registry`, the connections to `classify_links`),
//! - bundling cuts one helper, one `Arc` param, one set of caller
//!   migration sites — the same shape that S3 (6/N) used for
//!   `cached_calculation_registry` + `cached_frame_registry` inside the
//!   `EvalContext` seed.
//!
//! Three variants mirror `eval_context` and `precompiled_constraints`:
//!
//! - `file_port_flow_runtime(db, source_file)` — single-file (no
//!   workspace, no library overlay).
//! - `workspace_port_flow_runtime(db, pfs)` — workspace-merged (no
//!   library overlay).
//! - `workspace_port_flow_runtime_with_library(db, pfs, lib)` —
//!   workspace-merged with library overlay (the default for IDE / sim-app).
//!
//! All three use `build_port_flow_resources` from `sysml_runtime::flows`.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_runtime::flows::{build_port_flow_resources, PortFlowResources};

use crate::analysis::{elaborate_workspace, elaborate_workspace_with_library};
use crate::parse;
use crate::project_inputs::ProjectFileSet;
use crate::resolution::LibraryGraph;
use crate::source::SourceFile;
use crate::Db;

/// Salsa-cached port+flow runtime resources.
///
/// Wraps `Arc<PortFlowResources>` with pointer-identity equality so salsa
/// can memoize the value even though the inner type embeds `PortRegistry`
/// (`HashMap` — not `Eq`).
#[derive(Clone, Debug)]
pub struct CachedPortFlowRuntime(Arc<PortFlowResources>);

impl CachedPortFlowRuntime {
    fn new(resources: PortFlowResources) -> Self {
        Self(Arc::new(resources))
    }

    /// Borrow the inner `PortFlowResources`.
    pub fn resources(&self) -> &PortFlowResources {
        &self.0
    }

    /// Clone the inner `Arc<PortFlowResources>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<PortFlowResources> {
        Arc::clone(&self.0)
    }
}

salsa_arc_wrapper!(identity, CachedPortFlowRuntime, PortFlowResources);

/// Build port+flow resources for a single-file model graph.
///
/// Depends on: `parse_file()` (Layer 1).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_port_flow_runtime(
    db: &dyn Db,
    source_file: SourceFile,
) -> CachedPortFlowRuntime {
    let parsed = parse::parse_file(db, source_file);
    let resources = build_port_flow_resources(parsed.graph());
    CachedPortFlowRuntime::new(resources)
}

/// Build port+flow resources for a workspace-merged graph (no library overlay).
///
/// Depends on: `elaborate_workspace()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_port_flow_runtime(
    db: &dyn Db,
    pfs: ProjectFileSet,
) -> CachedPortFlowRuntime {
    let elaborated = elaborate_workspace(db, pfs);
    let resources = build_port_flow_resources(elaborated.graph());
    CachedPortFlowRuntime::new(resources)
}

/// Build port+flow resources for a workspace-merged graph with library overlay.
///
/// Depends on: `elaborate_workspace_with_library()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn workspace_port_flow_runtime_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
) -> CachedPortFlowRuntime {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    let resources = build_port_flow_resources(elaborated.graph());
    CachedPortFlowRuntime::new(resources)
}

/// Best-shape dispatcher — Some(lib) routes to ..._with_library, None to bare workspace.
// TODO(post-collapse): inline dispatch-parity tests
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_port_flow_runtime_best(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: Option<LibraryGraph>,
) -> CachedPortFlowRuntime {
    match library {
        Some(lib) => workspace_port_flow_runtime_with_library(db, pfs, lib),
        None => workspace_port_flow_runtime(db, pfs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::AnalysisHost;

    #[test]
    fn file_port_flow_runtime_caches_across_calls() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { port def WaterPort { in flowRate : Real; } \
             part def Tank { port out : WaterPort; } }"
                .to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let r1 = file_port_flow_runtime(analysis.db(), sf);
        let r2 = file_port_flow_runtime(analysis.db(), sf);

        // Salsa returns the same Arc on cache hits.
        assert!(Arc::ptr_eq(&r1.0, &r2.0));
    }

    #[test]
    fn file_port_flow_runtime_invalidates_on_content_change() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package Foo { port def InPort { in x : Real; } \
             part def A { port p : InPort; } }"
                .to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");
        let r1 = file_port_flow_runtime(host.analysis().db(), sf).arc();

        host.set_file_content(
            "test.sysml",
            "package Bar { port def OutPort { out y : Boolean; } \
             part def B { port q : OutPort; } }"
                .to_string(),
        );
        let sf2 = host.source_file(id).expect("source file still exists");
        let r2 = file_port_flow_runtime(host.analysis().db(), sf2).arc();

        // Different content → different Arc.
        assert!(!Arc::ptr_eq(&r1, &r2));
    }
}
