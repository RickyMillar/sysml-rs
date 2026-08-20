//! Tracked queries for the interaction descriptors (Bucket 1.5).
//!
//! The interaction map (`sysml_diagram::build_interaction_map`) projects each
//! element's renderer-agnostic semantic affordances (the go-to-definition target
//! today) into a node-id-keyed sidecar. Like the text-map (1.6) it is **not**
//! keyed by a view request — it is a property of the model graph, so it is built
//! once per `(graph)` and shared across every view (the cheaper standalone query
//! the `ViewModel` holds a ref into). Pure function of the elaborated graph.
//!
//! Command/label resolution is a thin overlay applied by the service layer
//! (Bucket 1.7) where the `#[service_command]` registry lives — it is NOT part of
//! this salsa-cached, session-context-free artifact.
//!
//! Three variants mirror the rest of the crate:
//! - `file_*` — single-file.
//! - `workspace_*` — workspace-merged (no library overlay).
//! - `workspace_*_with_library` — workspace-merged with library overlay.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_diagram::{build_interaction_map, InteractionMap};

use crate::analysis::{elaborate_file_best, elaborate_workspace, elaborate_workspace_with_library};
use crate::project_inputs::ProjectFileSet;
use crate::resolution::LibraryGraph;
use crate::source::SourceFile;
use crate::Db;

/// Salsa-cached [`InteractionMap`] for an unchanged elaborated graph.
///
/// Wraps `Arc<InteractionMap>` with pointer-identity equality (same rationale as
/// [`crate::text_map::CachedTextMap`]).
#[derive(Clone, Debug)]
pub struct CachedInteractionMap(Arc<InteractionMap>);

impl CachedInteractionMap {
    fn new(map: InteractionMap) -> Self {
        Self(Arc::new(map))
    }

    /// Borrow the inner [`InteractionMap`].
    pub fn interaction_map(&self) -> &InteractionMap {
        &self.0
    }

    /// Clone the inner `Arc<InteractionMap>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<InteractionMap> {
        Arc::clone(&self.0)
    }
}

salsa_arc_wrapper!(identity, CachedInteractionMap, InteractionMap);

/// Interaction map for a single-file model graph.
///
/// Built on the **resolved + elaborated** single-file graph
/// (`elaborate_file_best`, not bare `parse_file`): the go-to-definition target
/// is a *resolved* typing, which only exists after resolution. Element ids are
/// unchanged by resolution, so the map still joins to the (possibly unresolved)
/// scene by id.
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_interaction_map(db: &dyn Db, source_file: SourceFile) -> CachedInteractionMap {
    let elaborated = elaborate_file_best(db, source_file, None, None);
    CachedInteractionMap::new(build_interaction_map(elaborated.graph()))
}

/// Interaction map for a workspace-merged graph (no library overlay).
/// Depends on `elaborate_workspace()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_interaction_map(db: &dyn Db, pfs: ProjectFileSet) -> CachedInteractionMap {
    let elaborated = elaborate_workspace(db, pfs);
    CachedInteractionMap::new(build_interaction_map(elaborated.graph()))
}

/// Interaction map for a workspace-merged graph with library overlay.
/// Depends on `elaborate_workspace_with_library()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn workspace_interaction_map_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
) -> CachedInteractionMap {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    CachedInteractionMap::new(build_interaction_map(elaborated.graph()))
}

/// Best-shape dispatcher — `Some(lib)` routes to `..._with_library`, `None` to
/// the bare workspace query.
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_interaction_map_best(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: Option<LibraryGraph>,
) -> CachedInteractionMap {
    match library {
        Some(lib) => workspace_interaction_map_with_library(db, pfs, lib),
        None => workspace_interaction_map(db, pfs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::AnalysisHost;

    #[test]
    fn file_interaction_map_caches_and_resolves_typing() {
        // A typed usage (`p : Engine`) should resolve its go-to-definition target.
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def Engine; part p : Engine; }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let r1 = file_interaction_map(analysis.db(), sf);
        let r2 = file_interaction_map(analysis.db(), sf);
        assert!(Arc::ptr_eq(&r1.0, &r2.0), "should memoize");

        let map = r1.interaction_map();
        assert!(
            !map.is_empty(),
            "the typed usage `p : Engine` should produce a go-to-definition entry"
        );
        // Every stored entry is a real go-to-definition target.
        assert!(map.iter().all(|(_, e)| e.type_definition.is_some()));
    }

    #[test]
    fn file_interaction_map_invalidates_on_content_change() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def A; part a : A; }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");
        let r1 = file_interaction_map(host.analysis().db(), sf).arc();

        host.set_file_content(
            "test.sysml",
            "package P { part def A; part def B; part a : A; part b : B; }".to_string(),
        );
        let sf2 = host.source_file(id).expect("source file still exists");
        let r2 = file_interaction_map(host.analysis().db(), sf2).arc();

        assert!(!Arc::ptr_eq(&r1, &r2));
    }
}
