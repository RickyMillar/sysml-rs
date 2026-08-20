//! Tracked queries for ownership-tree `descendants` lookup.
//!
//! Pre-S2.T17 (4/N) every `service.descendants(uri, id)` call walked
//! the graph from the requested element via repeated `children_of`
//! lookups, cloning into an owned `Vec<Element>` for serialization.
//! The walk is bounded by subtree size (small per call) but the
//! command is invoked frequently — once per simulation-app subtree
//! expand, once per bulk-export pass, once per typed-feature
//! enumeration. Pre-T17 the result was discarded every time;
//! post-T17 it's salsa-cached per `(input, library, parse, id)`
//! tuple.
//!
//! Three variants mirror `eval_context` / `element_index`:
//!
//! - `file_descendants(db, sf, id)` — single-file mode (no
//!   workspace, no library merge).
//! - `workspace_descendants(db, pfs, id)` — workspace-merged
//!   (no library).
//! - `workspace_descendants_with_library(db, pfs, lib, id)` —
//!   workspace + library overlay.
//!
//! Result type `CachedDescendants` wraps `Arc<Vec<Element>>` with
//! pointer-identity equality (via `salsa_arc_wrapper!(identity, …)`);
//! salsa returns the same Arc on cache hits. `Element` itself isn't
//! `Eq` (embeds property maps and span vecs) but identity equality
//! is sufficient for salsa within a revision.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_core::{query, Element, ElementId, ModelGraph};

use crate::analysis::{elaborate_workspace, elaborate_workspace_with_library};
use crate::parse;
use crate::project_inputs::ProjectFileSet;
use crate::resolution::LibraryGraph;
use crate::source::SourceFile;
use crate::Db;

/// Salsa-cached `Vec<Element>` snapshot of an element's descendants.
///
/// Wraps `Arc<Vec<Element>>` with pointer-identity equality so salsa
/// can memoize the value across queries.
#[derive(Clone, Debug)]
pub struct CachedDescendants(Arc<Vec<Element>>);

impl CachedDescendants {
    fn new(elements: Vec<Element>) -> Self {
        Self(Arc::new(elements))
    }

    /// Borrow the inner element list.
    pub fn elements(&self) -> &[Element] {
        &self.0
    }

    /// Clone the inner `Arc<Vec<Element>>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<Vec<Element>> {
        Arc::clone(&self.0)
    }

    /// Clone the inner `Vec<Element>` (deep clone — only when callers
    /// need ownership).
    pub fn to_vec(&self) -> Vec<Element> {
        (*self.0).clone()
    }
}

salsa_arc_wrapper!(identity, CachedDescendants, Vec<Element>);

fn collect_descendants(graph: &ModelGraph, id: &ElementId) -> Vec<Element> {
    query::descendants(graph, id).into_iter().cloned().collect()
}

/// Build a descendants list for a single-file model graph.
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_descendants(db: &dyn Db, sf: SourceFile, id: ElementId) -> CachedDescendants {
    let parsed = parse::parse_file(db, sf);
    CachedDescendants::new(collect_descendants(parsed.graph(), &id))
}

/// Build a descendants list for the workspace-merged graph (no library).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_descendants(
    db: &dyn Db,
    pfs: ProjectFileSet,
    id: ElementId,
) -> CachedDescendants {
    let elaborated = elaborate_workspace(db, pfs);
    CachedDescendants::new(collect_descendants(elaborated.graph(), &id))
}

/// Build a descendants list for the workspace-merged graph with the
/// standard library merged in.
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn workspace_descendants_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
    id: ElementId,
) -> CachedDescendants {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    CachedDescendants::new(collect_descendants(elaborated.graph(), &id))
}

/// Best-shape dispatcher — Some(lib) routes to ..._with_library, None to bare workspace.
// TODO(post-collapse): inline dispatch-parity tests
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_descendants_best(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: Option<LibraryGraph>,
    id: ElementId,
) -> CachedDescendants {
    match library {
        Some(lib) => workspace_descendants_with_library(db, pfs, lib, id),
        None => workspace_descendants(db, pfs, id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::AnalysisHost;
    use crate::parse::parse_file;

    fn first_element_id(host: &AnalysisHost, sf: SourceFile) -> ElementId {
        let parsed = parse_file(host.db(), sf);
        parsed
            .graph()
            .elements
            .keys()
            .next()
            .expect("graph has at least one element")
            .clone()
    }

    #[test]
    fn file_descendants_caches_across_calls() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package Foo { part def Bar; }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");
        let root = first_element_id(&host, sf);

        let analysis = host.analysis();
        let d1 = file_descendants(analysis.db(), sf, root.clone());
        let d2 = file_descendants(analysis.db(), sf, root);

        assert!(Arc::ptr_eq(&d1.0, &d2.0));
    }

    #[test]
    fn file_descendants_invalidates_on_content_change() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package Foo { part def Bar; }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");
        let root = first_element_id(&host, sf);
        let d1 = file_descendants(host.analysis().db(), sf, root.clone()).arc();

        host.set_file_content(
            "test.sysml",
            "package Foo { part def Baz; }".to_string(),
        );
        let sf2 = host.source_file(id).expect("source file still exists");
        let root2 = first_element_id(&host, sf2);
        let d2 = file_descendants(host.analysis().db(), sf2, root2).arc();

        assert!(!Arc::ptr_eq(&d1, &d2));
    }

    #[test]
    fn file_descendants_distinguishes_ids() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package Foo { part def Bar; part def Baz; }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let parsed = parse_file(analysis.db(), sf);
        let mut ids = parsed.graph().elements.keys();
        let id_a = ids.next().expect("at least one element").clone();
        let id_b = ids
            .find(|id| **id != id_a)
            .expect("at least two elements")
            .clone();

        let d_a = file_descendants(analysis.db(), sf, id_a);
        let d_b = file_descendants(analysis.db(), sf, id_b);

        // Different inputs → distinct salsa slots → distinct Arcs.
        assert!(!Arc::ptr_eq(&d_a.0, &d_b.0));
    }

    #[test]
    fn file_descendants_walks_subtree() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package Foo { part def Bar { attribute x; } }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let analysis = host.analysis();
        let parsed = parse_file(analysis.db(), sf);
        // Find the root element (first by iteration, no guarantee on
        // which but the test only asserts the descendants list isn't
        // entirely empty across SOME root).
        let any_with_children = parsed
            .graph()
            .elements
            .keys()
            .find(|id| parsed.graph().children_of(id).next().is_some())
            .expect("some element has children")
            .clone();

        let d = file_descendants(analysis.db(), sf, any_with_children);
        assert!(
            !d.elements().is_empty(),
            "expected non-empty descendants for an element known to have children"
        );
    }
}
