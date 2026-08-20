//! Tracked queries for the gated-expression bundle consumed by
//! `Orchestrator::add_computed_expression`.
//!
//! The orchestrator's `computed_expressions` field is populated from two
//! graph walks inside `build_workspace_orchestrator`:
//!
//! - `ModelCompiler::detect_computed_expressions` — direct attribute
//!   `= expr` bindings.
//! - `ModelCompiler::detect_instance_scoped_expressions` — instance-multiplied
//!   attribute bindings (e.g., `circuit1.tripped`, `circuit2.tripped`, … from
//!   one `CircuitPath.tripped = bimetalTemp >= …` definition).
//!
//! Both walks are pure graph derivatives, and each entry pays one extra
//! `ode_builder::parse_derivative` call to turn the RHS string into
//! `ExprIR`. On stdlib-heavy workspaces (large multi-circuit models, ten multiplied
//! circuits with per-instance computed attributes), the parse alone costs
//! a few milliseconds per `build_workspace_orchestrator` and was re-paid
//! on every simulation start.
//!
//! Per ADR-011 §3 row `cached_gated_expressions` (RT-16), this is the
//! natural tracked-query target. The two upstream walks plus the parse
//! bundle into a single cached `Vec<(String, ExprIR)>` because:
//!
//! - both walks share the same graph-revision invalidation key,
//! - both feed into the same downstream consumer
//!   (`Orchestrator::add_computed_expression`),
//! - bundling cuts one helper, one `Arc` param, one set of caller
//!   migration sites — the same shape that S3 (6/N) used for
//!   `cached_calculation_registry` + `cached_frame_registry` and
//!   (7/N) used for `cached_port_registry` + `cached_flow_ir`.
//!
//! Variants:
//!
//! - `file_gated_expressions(db, source_file)` — single-file.
//! - `workspace_gated_expressions(db, pfs)` — workspace-merged, USER-only
//!   (no library overlay).
//! - `workspace_gated_expressions_best(db, pfs, library)` — the dispatcher
//!   callers use. Gated/computed expressions are user-model derived bindings,
//!   so this always builds from the user-only graph and ignores `library`
//!   (see the function doc for the full rationale). There is intentionally no
//!   `_with_library` variant — the library overlay is never a valid source.
//!
//! All call `sysml_runtime::compiler::build_gated_expressions` on the
//! elaborated graph.
//!
//! Note on semantics divergence: the cached walk skips the SM/ODE
//! reachability filter that `expand_part_instances` applies in the
//! in-place path. The extra prefixed entries are inert — they sit in
//! the orchestrator's `computed_expressions` HashMap and never fire
//! because their target variables are not part of any executing
//! subsystem. The rationale is documented above
//! `build_gated_expressions` in `sysml_runtime::compiler`.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_runtime::compiler::{build_gated_expressions, GatedExprSpec};

use crate::analysis::elaborate_workspace;
use crate::parse;
use crate::project_inputs::ProjectFileSet;
use crate::resolution::LibraryGraph;
use crate::source::SourceFile;
use crate::Db;

/// Salsa-cached gated-expression bundle.
///
/// Wraps `Arc<Vec<GatedExprSpec>>` with pointer-identity equality so
/// salsa can memoize the value even though the inner `ExprIR` does not
/// implement `Eq`. Each [`GatedExprSpec`] carries its instance
/// `scope_prefix` (RSC-4.2 C.4) so the orchestrator binds instance-scoped
/// RHS expressions to their instance's slots instead of text-prefixing.
#[derive(Clone, Debug)]
pub struct CachedGatedExpressions(Arc<Vec<GatedExprSpec>>);

impl CachedGatedExpressions {
    fn new(entries: Vec<GatedExprSpec>) -> Self {
        Self(Arc::new(entries))
    }

    /// Borrow the inner `Vec<GatedExprSpec>`.
    pub fn entries(&self) -> &[GatedExprSpec] {
        &self.0
    }

    /// Clone the inner `Arc<Vec<GatedExprSpec>>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<Vec<GatedExprSpec>> {
        Arc::clone(&self.0)
    }
}

salsa_arc_wrapper!(identity, CachedGatedExpressions, Vec<GatedExprSpec>);

/// Build gated expressions for a single-file model graph.
///
/// Depends on: `parse_file()` (Layer 1).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_gated_expressions(
    db: &dyn Db,
    source_file: SourceFile,
) -> CachedGatedExpressions {
    let parsed = parse::parse_file(db, source_file);
    let entries = build_gated_expressions(parsed.graph());
    CachedGatedExpressions::new(entries)
}

/// Build gated expressions for a workspace-merged graph (no library overlay).
///
/// Depends on: `elaborate_workspace()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_gated_expressions(
    db: &dyn Db,
    pfs: ProjectFileSet,
) -> CachedGatedExpressions {
    let elaborated = elaborate_workspace(db, pfs);
    let entries = build_gated_expressions(elaborated.graph());
    CachedGatedExpressions::new(entries)
}

/// Best-shape dispatcher for gated/computed expressions.
///
/// Gated/computed expressions are orchestrator per-tick derived bindings, and
/// those are by definition USER-model attributes (`= expr`). The standard
/// library overlay is never a valid source: its derived-unit and analysis
/// templates (`attribute <Pa> pascal = N/m^2`, `MaximizeObjective.best =
/// alternatives->maximize{…}`) are definitional metadata whose RHS reference
/// unit symbols / calc-locals that resolve to no runtime slot. Building from
/// the library-merged graph collected those and produced `RS003` hard errors;
/// worse, a per-element library filter on the merged graph cannot be made
/// exhaustive (a library element with a broken owner cache, e.g. orphaned
/// `TradeStudies::MaximizeObjective`, slips through any owner-walk).
///
/// So we always build from the user-only elaborated workspace graph
/// (`elaborate_workspace`, no library merge) regardless of whether a library
/// is loaded. Collection needs only expression text + node-kind classification,
/// both of which live entirely in the user graph; library type resolution
/// happens later, at evaluation time against the merged runtime context, not
/// at collection time. `library` is therefore intentionally ignored here.
/// (Steward ruling: scope correction, not a workaround — the merged graph was
/// always the wrong input for this query.)
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_gated_expressions_best(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: Option<LibraryGraph>,
) -> CachedGatedExpressions {
    let _ = library;
    workspace_gated_expressions(db, pfs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::AnalysisHost;

    #[test]
    fn file_gated_expressions_caches_across_calls() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def Tank { attribute level : Real; \
             attribute full = level >= 100; } }"
                .to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let r1 = file_gated_expressions(analysis.db(), sf);
        let r2 = file_gated_expressions(analysis.db(), sf);

        // Salsa returns the same Arc on cache hits.
        assert!(Arc::ptr_eq(&r1.0, &r2.0));
    }

    #[test]
    fn file_gated_expressions_invalidates_on_content_change() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package Foo { part def A { attribute x : Real; \
             attribute y = x + 1; } }"
                .to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");
        let r1 = file_gated_expressions(host.analysis().db(), sf).arc();

        host.set_file_content(
            "test.sysml",
            "package Bar { part def B { attribute p : Real; \
             attribute q = p * 2; } }"
                .to_string(),
        );
        let sf2 = host.source_file(id).expect("source file still exists");
        let r2 = file_gated_expressions(host.analysis().db(), sf2).arc();

        // Different content → different Arc.
        assert!(!Arc::ptr_eq(&r1, &r2));
    }
}
