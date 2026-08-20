//! Tracked queries for `model_tree` projection.
//!
//! Pre-S2.T17 (3/N) every `service.model_tree(uri, depth, view)` call
//! re-walked the whole model graph: extracted bounds for every
//! AttributeUsage, recursed through every root, dedup-by-(name,kind)
//! sorted by archetype rank, and minted fresh UUIDs for typed-def
//! inlining duplicates. The walk is O(elements + relationships) and
//! gets called per IDE outline refresh, per Run-target dropdown,
//! per simulation-app tree render. Pre-T17 the result was discarded
//! every time; post-T17 it's salsa-cached per (input, library,
//! parse, depth, view) tuple.
//!
//! Five variants mirror the dual-graph (graph, resolver) shape of
//! `model_tree_with_resolver`:
//!
//! - `file_model_tree(db, sf, depth, view)` — single-file mode; the
//!   file graph is its own resolver.
//! - `workspace_model_tree(db, pfs, depth, view)` — workspace-merged
//!   mode (no library); workspace graph is its own resolver.
//! - `workspace_model_tree_with_library(db, pfs, lib, depth, view)` —
//!   workspace + library, used as both source and resolver.
//! - `file_in_workspace_model_tree(db, sf, pfs, depth, view)` —
//!   single-file roots, workspace resolver. Lets per-file outlines
//!   surface cross-file typed-definition children without leaking
//!   the other files' roots.
//! - `file_in_workspace_model_tree_with_library(db, sf, pfs, lib,
//!   depth, view)` — same with library overlay.
//!
//! The result type `CachedModelTree` wraps `Arc<Vec<TreeNode>>` with
//! pointer-identity equality; salsa returns the same Arc on cache
//! hits so downstream consumers benefit transparently. `TreeNode`
//! itself is not `Eq` (embeds `Vec<TreeNode>`, `Option<String>`,
//! enums) but identity equality is sufficient for salsa within a
//! revision.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_ide_db::{
    elaborate_workspace_best, parse_file, Db, LibraryGraph, ProjectFileSet, SourceFile,
};

use crate::query::{model_tree_with_resolver, TreeView};
use crate::types::TreeNode;

/// Salsa-cached `Vec<TreeNode>` snapshot.
///
/// Wraps `Arc<Vec<TreeNode>>` with pointer-identity equality so salsa
/// can memoize the value across queries even though `TreeNode` itself
/// isn't `Eq` (recursive structure containing options, enums, and a
/// nested `Vec<TreeNode>` of itself).
#[derive(Clone, Debug)]
pub struct CachedModelTree(Arc<Vec<TreeNode>>);

impl CachedModelTree {
    fn new(nodes: Vec<TreeNode>) -> Self {
        Self(Arc::new(nodes))
    }

    /// Borrow the inner tree.
    pub fn nodes(&self) -> &[TreeNode] {
        &self.0
    }

    /// Clone the inner `Arc<Vec<TreeNode>>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<Vec<TreeNode>> {
        Arc::clone(&self.0)
    }

    /// Clone the inner `Vec<TreeNode>` (deep clone — only when callers
    /// need ownership).
    pub fn to_vec(&self) -> Vec<TreeNode> {
        (*self.0).clone()
    }
}

impl PartialEq for CachedModelTree {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for CachedModelTree {}

impl Hash for CachedModelTree {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

/// Build a model tree for a single-file model graph.
///
/// Both source-of-roots and type-resolver are the same single-file
/// graph. Used when no workspace `ProjectFileSet` is loaded.
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn file_model_tree(
    db: &dyn Db,
    sf: SourceFile,
    max_depth: Option<usize>,
    view: TreeView,
) -> CachedModelTree {
    let parsed = parse_file(db, sf);
    let graph = parsed.graph();
    CachedModelTree::new(model_tree_with_resolver(graph, graph, max_depth, view))
}

/// Build a model tree for the workspace-merged graph (no library).
///
/// Both source-of-roots and resolver are the workspace graph.
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_model_tree(
    db: &dyn Db,
    pfs: ProjectFileSet,
    max_depth: Option<usize>,
    view: TreeView,
) -> CachedModelTree {
    let elaborated = elaborate_workspace_best(db, pfs, None);
    let graph = elaborated.graph();
    CachedModelTree::new(model_tree_with_resolver(graph, graph, max_depth, view))
}

/// Build a model tree for the workspace-merged graph with the
/// standard library merged in. Both source-of-roots and resolver
/// are the workspace+library graph.
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn workspace_model_tree_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
    max_depth: Option<usize>,
    view: TreeView,
) -> CachedModelTree {
    let elaborated = elaborate_workspace_best(db, pfs, Some(library));
    let graph = elaborated.graph();
    CachedModelTree::new(model_tree_with_resolver(graph, graph, max_depth, view))
}

/// Dispatcher: route to `workspace_model_tree_with_library` or
/// `workspace_model_tree` based on `library`.
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_model_tree_best(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: Option<LibraryGraph>,
    max_depth: Option<usize>,
    view: TreeView,
) -> CachedModelTree {
    match library {
        Some(lib) => workspace_model_tree_with_library(db, pfs, lib, max_depth, view),
        None => workspace_model_tree(db, pfs, max_depth, view),
    }
}

/// Build a model tree from a single-file's roots, but using the
/// workspace-merged graph as the type-resolver.
///
/// This is the canonical "outline for one file in a multi-file
/// workspace" projection: roots come from `sf` (so the user sees
/// only their file's elements) but a usage typed by a definition
/// in another workspace file still inlines its typed-def children.
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn file_in_workspace_model_tree(
    db: &dyn Db,
    sf: SourceFile,
    pfs: ProjectFileSet,
    max_depth: Option<usize>,
    view: TreeView,
) -> CachedModelTree {
    let parsed = parse_file(db, sf);
    let elaborated = elaborate_workspace_best(db, pfs, None);
    CachedModelTree::new(model_tree_with_resolver(
        parsed.graph(),
        elaborated.graph(),
        max_depth,
        view,
    ))
}

/// Like [`file_in_workspace_model_tree`] but with the standard
/// library merged into the resolver graph.
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn file_in_workspace_model_tree_with_library(
    db: &dyn Db,
    sf: SourceFile,
    pfs: ProjectFileSet,
    library: LibraryGraph,
    max_depth: Option<usize>,
    view: TreeView,
) -> CachedModelTree {
    let parsed = parse_file(db, sf);
    let elaborated = elaborate_workspace_best(db, pfs, Some(library));
    CachedModelTree::new(model_tree_with_resolver(
        parsed.graph(),
        elaborated.graph(),
        max_depth,
        view,
    ))
}

/// Dispatcher: route to one of `file_in_workspace_model_tree_with_library`,
/// `file_in_workspace_model_tree`, or `file_model_tree` based on
/// the presence of project_files / library inputs.
///
/// When project_files is None, library has no effect — single-file mode.
#[tracing::instrument(level = "debug", skip(db, project_files, library))]
#[salsa::tracked]
pub fn file_in_workspace_model_tree_best(
    db: &dyn Db,
    source_file: SourceFile,
    project_files: Option<ProjectFileSet>,
    library: Option<LibraryGraph>,
    max_depth: Option<usize>,
    view: TreeView,
) -> CachedModelTree {
    match (project_files, library) {
        (Some(pfs), Some(lib)) => {
            file_in_workspace_model_tree_with_library(db, source_file, pfs, lib, max_depth, view)
        }
        (Some(pfs), None) => file_in_workspace_model_tree(db, source_file, pfs, max_depth, view),
        (None, _) => file_model_tree(db, source_file, max_depth, view),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_ide_db::AnalysisHost;

    #[test]
    fn file_model_tree_caches_across_calls() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content("test.sysml", "package Foo {}".to_string());
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let t1 = file_model_tree(analysis.db(), sf, None, TreeView::UserFacing);
        let t2 = file_model_tree(analysis.db(), sf, None, TreeView::UserFacing);

        assert!(Arc::ptr_eq(&t1.0, &t2.0));
    }

    #[test]
    fn file_model_tree_invalidates_on_content_change() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content("test.sysml", "package Foo {}".to_string());
        let sf = host.source_file(id).expect("source file exists");
        let t1 = file_model_tree(host.analysis().db(), sf, None, TreeView::UserFacing).arc();

        host.set_file_content("test.sysml", "package Bar {}".to_string());
        let sf2 = host.source_file(id).expect("source file still exists");
        let t2 = file_model_tree(host.analysis().db(), sf2, None, TreeView::UserFacing).arc();

        assert!(!Arc::ptr_eq(&t1, &t2));
    }

    #[test]
    fn file_model_tree_distinguishes_views() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content("test.sysml", "package Foo {}".to_string());
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let user = file_model_tree(analysis.db(), sf, None, TreeView::UserFacing);
        let full = file_model_tree(analysis.db(), sf, None, TreeView::Full);

        // Different view → distinct salsa slots → distinct Arcs.
        assert!(!Arc::ptr_eq(&user.0, &full.0));
    }

    #[test]
    fn file_model_tree_distinguishes_depth() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package Foo { part def Bar { part baz : Bar; } }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let unlimited = file_model_tree(analysis.db(), sf, None, TreeView::UserFacing);
        let shallow = file_model_tree(analysis.db(), sf, Some(1), TreeView::UserFacing);

        assert!(!Arc::ptr_eq(&unlimited.0, &shallow.0));
    }

    #[test]
    fn file_model_tree_produces_a_root() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content("test.sysml", "package Foo {}".to_string());
        let sf = host.source_file(id).expect("source file exists");

        let tree = file_model_tree(host.analysis().db(), sf, None, TreeView::UserFacing);
        assert!(
            !tree.nodes().is_empty(),
            "expected at least one root TreeNode, got empty tree"
        );
    }

    #[test]
    fn file_in_workspace_model_tree_best_none_routes_to_file_model_tree() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content("test.sysml", "package Foo {}".to_string());
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let dispatched = file_in_workspace_model_tree_best(
            analysis.db(),
            sf,
            None,
            None,
            None,
            TreeView::UserFacing,
        );
        let direct = file_model_tree(analysis.db(), sf, None, TreeView::UserFacing);

        // Dispatch parity: salsa returns the same memoized Arc for the
        // delegated call, so pointer-identity must hold.
        assert!(Arc::ptr_eq(&dispatched.0, &direct.0));
    }

    #[test]
    fn file_in_workspace_model_tree_best_none_library_ignored() {
        // When project_files is None, library has no effect — both
        // (None, None) and (None, Some(lib)) must route to the same
        // single-file path. We can only easily prove the (None, None)
        // arm here without spinning up a real LibraryGraph; the
        // (None, Some) arm is documented in the dispatcher doc-comment.
        let mut host = AnalysisHost::new();
        let id = host.set_file_content("test.sysml", "package Foo {}".to_string());
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let a =
            file_in_workspace_model_tree_best(analysis.db(), sf, None, None, None, TreeView::Full);
        let b = file_model_tree(analysis.db(), sf, None, TreeView::Full);
        assert!(Arc::ptr_eq(&a.0, &b.0));
    }

    // TODO(post-collapse): workspace_model_tree_best dispatch-parity
    // tests for Some(lib)/None arms, and file_in_workspace_model_tree_best
    // (Some,Some)/(Some,None) arms — needs a ProjectFileSet + LibraryGraph
    // setup (see open_context.rs::tests for the recipe).
}
