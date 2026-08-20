//! Tracked queries for `sysml_runtime::constraints::PrecompiledConstraintSet`.
//!
//! Constraint pre-compilation walks every `ConstraintUsage` /
//! `AssertConstraintUsage` / `ConstraintDefinition` in the model graph and
//! parses each expression to `ExprIR`. The walk is O(n + m·k) where `n` is
//! the element count, `m` is the constraint count and `k` is the average
//! expression depth. On stdlib-heavy workspaces (large multi-circuit models,
//! ~50–100 constraints) the precompile costs single-digit milliseconds
//! per call.
//!
//! Per ADR-011 §3 row `cached_precompiled_constraints` (replacing the
//! dormant `Orchestrator.PrecompiledConstraintSet` field RT-6), the
//! precompile is a pure graph derivative and a natural tracked-query
//! target. Once cached, every `simulate.start` /
//! `orchestrate.workspace.start` that wires constraint monitoring into the
//! orchestrator hits a single Arc clone instead of re-extracting and
//! re-parsing the full constraint set.
//!
//! Three variants mirror `eval_context`:
//!
//! - `file_precompiled_constraints(db, source_file)` — single-file
//!   precompile (no workspace, no library overlay).
//! - `workspace_precompiled_constraints(db, pfs)` — workspace-merged
//!   precompile (no library overlay).
//! - `workspace_precompiled_constraints_with_library(db, pfs, lib)` —
//!   workspace-merged with library overlay (the default for IDE / sim-app).
//!
//! All three use `extract_and_precompile` from `sysml_runtime::constraints`
//! — unfiltered (every constraint in the elaborated graph). Callers that
//! need to filter (e.g. `sysml.constraint.check` excludes library elements
//! and optionally scopes to a single file) do so on the cached result
//! rather than at extract time.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_runtime::constraints::{extract_and_precompile, PrecompiledConstraintSet};

use crate::analysis::{elaborate_workspace, elaborate_workspace_with_library};
use crate::parse;
use crate::project_inputs::ProjectFileSet;
use crate::resolution::LibraryGraph;
use crate::source::SourceFile;
use crate::Db;

/// Salsa-cached precompiled constraint set.
///
/// Wraps `Arc<PrecompiledConstraintSet>` with pointer-identity equality so
/// salsa can memoize the value even though the inner type is not `Eq`
/// (it embeds `TypedConstraint`s with `ExprIR` trees).
#[derive(Clone, Debug)]
pub struct CachedPrecompiledConstraintSet(Arc<PrecompiledConstraintSet>);

impl CachedPrecompiledConstraintSet {
    fn new(set: PrecompiledConstraintSet) -> Self {
        Self(Arc::new(set))
    }

    /// Borrow the inner `PrecompiledConstraintSet`.
    pub fn set(&self) -> &PrecompiledConstraintSet {
        &self.0
    }

    /// Clone the inner `Arc<PrecompiledConstraintSet>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<PrecompiledConstraintSet> {
        Arc::clone(&self.0)
    }
}

salsa_arc_wrapper!(
    identity,
    CachedPrecompiledConstraintSet,
    PrecompiledConstraintSet
);

/// Precompile constraints for a single-file model graph.
///
/// Depends on: `parse_file()` (Layer 1).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_precompiled_constraints(
    db: &dyn Db,
    source_file: SourceFile,
) -> CachedPrecompiledConstraintSet {
    let parsed = parse::parse_file(db, source_file);
    let set = extract_and_precompile(parsed.graph());
    CachedPrecompiledConstraintSet::new(set)
}

/// Precompile constraints for a workspace-merged graph (no library overlay).
///
/// Depends on: `elaborate_workspace()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_precompiled_constraints(
    db: &dyn Db,
    pfs: ProjectFileSet,
) -> CachedPrecompiledConstraintSet {
    let elaborated = elaborate_workspace(db, pfs);
    let set = extract_and_precompile(elaborated.graph());
    CachedPrecompiledConstraintSet::new(set)
}

/// Precompile constraints for a workspace-merged graph with library overlay.
///
/// Depends on: `elaborate_workspace_with_library()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn workspace_precompiled_constraints_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
) -> CachedPrecompiledConstraintSet {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    let set = extract_and_precompile(elaborated.graph());
    CachedPrecompiledConstraintSet::new(set)
}

/// Best-shape dispatcher — Some(lib) routes to ..._with_library, None to bare workspace.
// TODO(post-collapse): inline dispatch-parity tests
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_precompiled_constraints_best(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: Option<LibraryGraph>,
) -> CachedPrecompiledConstraintSet {
    match library {
        Some(lib) => workspace_precompiled_constraints_with_library(db, pfs, lib),
        None => workspace_precompiled_constraints(db, pfs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::AnalysisHost;

    #[test]
    fn file_precompiled_constraints_caches_across_calls() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def Vehicle { constraint c { 1 < 2 } } }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let s1 = file_precompiled_constraints(analysis.db(), sf);
        let s2 = file_precompiled_constraints(analysis.db(), sf);

        // Salsa returns the same Arc on cache hits.
        assert!(Arc::ptr_eq(&s1.0, &s2.0));
    }

    #[test]
    fn file_precompiled_constraints_invalidates_on_content_change() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package Foo { part def Vehicle { constraint c { 1 < 2 } } }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");
        let s1 = file_precompiled_constraints(host.analysis().db(), sf).arc();

        host.set_file_content(
            "test.sysml",
            "package Bar { part def Boat { constraint d { 5 < 10 } } }".to_string(),
        );
        let sf2 = host.source_file(id).expect("source file still exists");
        let s2 = file_precompiled_constraints(host.analysis().db(), sf2).arc();

        // Different content → different Arc.
        assert!(!Arc::ptr_eq(&s1, &s2));
    }
}
