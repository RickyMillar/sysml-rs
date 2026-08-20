//! Salsa-cached cross-file `ElementId -> Vec<RefSite>` reverse index.
//!
//! feature that needs "who references this element" — find-references,
//! rename, call-hierarchy, find-implementations, find-subsettings,
//! find-redefinitions — used to reinvent some cross-file walk: name-
//! based matching (unsound), grep, or N salsa queries per file. This
//! module collapses that into one workspace-merged scan, memoized by
//! salsa so per-file edits invalidate only the touched bucket.
//!
//! Variants mirror `element_index`:
//!   - `workspace_ref_index(db, pfs)` — workspace-merged (no library)
//!   - `workspace_ref_index_with_library(db, pfs, lib)` — workspace + lib
//!
//! A `file_ref_index` variant is intentionally absent: per-file
//! references can be recovered from the workspace index by filtering
//! `RefSite::file == uri`, and the workspace index is what
//! find-references / rename actually want anyway.
//!
//! Result type wraps `Arc<HashMap<…>>` with identity equality, same
//! `salsa_arc_wrapper!(identity, …)` shape as `CachedNameIndex` /
//! `CachedKindIndex`.
//!
//! ## What's indexed
//!
//! For every element in the workspace-merged graph:
//!   - emit one [`RefKind::Definition`] entry for the element's own
//!     declaration span (the first entry in `element.spans`).
//!   - if the element is a relationship, walk every `Value::Ref` prop
//!     and emit a [`RefKind::RelationshipTarget`] entry keyed on the
//!     target id, attaching the prop name and relationship kind so
//!     consumers can filter (e.g. find-redefinitions = only entries
//!     where `relationship_kind == Redefinition` and `prop_name ==
//!     "redefinedFeature"`).
//!
//! ## What's NOT indexed (Phase 2.5 follow-up)
//!
//! Free-standing `FeatureReferenceExpression` resolutions ("named
//! uses") — e.g. `attribute power = engine.power;` resolving the
//! `engine.power` chain back to a feature id. Those need a prop-list
//! walk we haven't enumerated yet. Defer until find-references on a
//! feature actually needs them.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use rustc_hash::FxHashSet;
use sysml_core::{ElementId, ElementKind, ModelGraph};

use crate::analysis::{elaborate_workspace, elaborate_workspace_with_library};
use crate::project_inputs::ProjectFileSet;
use crate::resolution::LibraryGraph;
use crate::Db;

/// What a [`RefSite`] represents about the relationship between its
/// `span` and the keyed `ElementId`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum RefKind {
    /// The keyed element's own decl span (first entry in
    /// [`Element::spans`](sysml_core::Element)).
    Definition,
    /// A relationship element whose `relationship_prop` on its own
    /// `props` is `Value::Ref(keyed_id)`. The site's `(file, start,
    /// end)` is the relationship's span (i.e. the *use* site in source).
    RelationshipTarget,
}

/// Where the indexed element came from in the merged graph. Lets
/// rename-across-project skip library shadows and lets import-health
/// distinguish workspace vs library origins without re-walking three
/// graphs (see `diagnostics.rs:337`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Provenance {
    /// Element lives in the workspace `ProjectFileSet`.
    Workspace,
    /// Element came from the standard-library overlay (only possible
    /// in the `*_with_library` variant of the query).
    Library,
}

/// One reference to an element.
///
/// `(file, start, end)` is a byte-offset span in the source file; the
/// transport converts to line/col via `position::offset_to_line_col`.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct RefSite {
    pub file: String,
    pub start: usize,
    pub end: usize,
    pub kind: RefKind,
    pub provenance: Provenance,
    /// For `RelationshipTarget` kind: the prop name on the relationship
    /// element (e.g. `"type"`, `"subsettedFeature"`,
    /// `"redefinedFeature"`). `None` for `Definition`.
    pub relationship_prop: Option<String>,
    /// For `RelationshipTarget` kind: the kind of the relationship
    /// element itself (FeatureTyping / Subsetting / Redefinition /
    /// Specialization / …). `None` for `Definition`. Carried so
    /// find-subsettings / find-redefinitions can filter without
    /// re-querying the graph.
    pub relationship_kind: Option<ElementKind>,
}

/// Salsa-cached `ElementId -> Vec<RefSite>` index.
///
/// Wraps `Arc<HashMap<…>>` with pointer-identity equality so salsa
/// returns the same `Arc` on cache hits.
#[derive(Clone, Debug)]
pub struct CachedRefIndex(Arc<HashMap<ElementId, Vec<RefSite>>>);

impl CachedRefIndex {
    fn new(map: HashMap<ElementId, Vec<RefSite>>) -> Self {
        Self(Arc::new(map))
    }

    /// Borrow the inner map.
    pub fn map(&self) -> &HashMap<ElementId, Vec<RefSite>> {
        &self.0
    }

    /// Clone the inner `Arc<HashMap<…>>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<HashMap<ElementId, Vec<RefSite>>> {
        Arc::clone(&self.0)
    }

    /// Look up reference sites for the given element id. Returns empty
    /// slice on miss.
    pub fn get(&self, id: &ElementId) -> &[RefSite] {
        self.0.get(id).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

salsa_arc_wrapper!(identity, CachedRefIndex, HashMap<ElementId, Vec<RefSite>>);

/// Build the reverse index from a model graph. Pure function — testable
/// without salsa. `library_ids` tags elements that came from the
/// library overlay; pass an empty set for the no-library variant.
fn build_ref_index(
    graph: &ModelGraph,
    library_ids: &FxHashSet<ElementId>,
) -> HashMap<ElementId, Vec<RefSite>> {
    let mut map: HashMap<ElementId, Vec<RefSite>> = HashMap::new();

    let provenance_of = |id: &ElementId| {
        if library_ids.contains(id) {
            Provenance::Library
        } else {
            Provenance::Workspace
        }
    };

    for el in graph.elements.values() {
        // 1. Self-definition: keyed on the element's own id, span = its
        //    decl site. Skip elements without spans (synthetic).
        if let Some(span) = el.spans.first() {
            map.entry(el.id.clone()).or_default().push(RefSite {
                file: span.file.clone(),
                start: span.start,
                end: span.end,
                kind: RefKind::Definition,
                provenance: provenance_of(&el.id),
                relationship_prop: None,
                relationship_kind: None,
            });
        }

        // 2. Relationships: walk every Value::Ref prop and emit a
        //    RelationshipTarget entry keyed on the target id. The span
        //    used is the RELATIONSHIP's own span (the use site in
        //    source, e.g. the `:` of `part p : Type`).
        if el.kind.is_relationship() {
            let Some(rel_span) = el.spans.first() else {
                continue;
            };
            for (prop_name, value) in el.props.iter() {
                let Some(target_id) = value.as_ref() else {
                    continue;
                };
                // Skip self-references (defensive; would never happen
                // in well-formed graphs but cheap to guard).
                if target_id == &el.id {
                    continue;
                }
                map.entry(target_id.clone()).or_default().push(RefSite {
                    file: rel_span.file.clone(),
                    start: rel_span.start,
                    end: rel_span.end,
                    kind: RefKind::RelationshipTarget,
                    provenance: provenance_of(target_id),
                    relationship_prop: Some(prop_name.to_string()),
                    relationship_kind: Some(el.kind.clone()),
                });
            }
        }
    }

    map
}

/// Build the reverse index over the workspace-merged graph (no library).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn workspace_ref_index(db: &dyn Db, pfs: ProjectFileSet) -> CachedRefIndex {
    let elaborated = elaborate_workspace(db, pfs);
    let empty: FxHashSet<ElementId> = FxHashSet::default();
    CachedRefIndex::new(build_ref_index(elaborated.graph(), &empty))
}

/// Build the reverse index over the workspace-merged graph with the
/// library overlay merged in. Library-origin elements are tagged
/// `Provenance::Library` so consumers (rename, references) can filter.
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_ref_index_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
) -> CachedRefIndex {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    let lib_data = library.data(db);
    let library_ids = lib_data.element_ids();
    CachedRefIndex::new(build_ref_index(elaborated.graph(), library_ids))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::AnalysisHost;
    use crate::project_inputs::PROJECT_KIND_DISCOVERED;
    use sysml_project::ProjectHandle;

    /// Build a workspace with the given (uri, content) files under one
    /// project. Mirrors the pattern in host.rs::tests (make_project +
    /// set_file_content_in_project + add_project_file_set).
    fn host_with_workspace(files: &[(&str, &str)]) -> (AnalysisHost, ProjectFileSet) {
        let mut host = AnalysisHost::new();
        let project = sysml_project::Project {
            id: ProjectHandle(42),
            info: sysml_project::ProjectInfo {
                name: "ref-index-test".to_string(),
                description: None,
                version: "1.0.0".to_string(),
                topic: Vec::new(),
                usage: Vec::new(),
            },
            meta: None,
            root: sysml_project::ProjectRoot::InMemory,
        };
        host.load_project(project);
        let pid = ProjectHandle(42);

        let mut source_files = Vec::new();
        for (uri, content) in files {
            host.set_file_content_in_project(uri, (*content).to_string(), pid);
            let file_id = host.file_id(uri).expect("file_id after set_file_content_in_project");
            let sf = host.source_file(file_id).expect("source_file");
            source_files.push(sf);
        }

        let pfs = ProjectFileSet::new(
            host.db(),
            pid.0,
            std::sync::Arc::new(source_files),
            PROJECT_KIND_DISCOVERED,
        );
        host.add_project_file_set(pfs);
        (host, pfs)
    }

    #[test]
    fn workspace_ref_index_emits_definition_entries() {
        let (host, pfs) = host_with_workspace(&[("file:///test.sysml", "part def Widget;")]);

        let idx = workspace_ref_index(host.analysis().db(), pfs);
        let map = idx.map();

        let has_def = map
            .values()
            .flatten()
            .any(|s| matches!(s.kind, RefKind::Definition));
        assert!(has_def, "expected at least one Definition entry, got {map:?}");
    }

    #[test]
    fn workspace_ref_index_caches_across_calls() {
        let (host, pfs) = host_with_workspace(&[("file:///test.sysml", "part def Widget;")]);

        let analysis = host.analysis();
        let idx1 = workspace_ref_index(analysis.db(), pfs);
        let idx2 = workspace_ref_index(analysis.db(), pfs);

        assert!(Arc::ptr_eq(&idx1.0, &idx2.0));
    }

    #[test]
    fn workspace_ref_index_collects_cross_file_relationship_targets() {
        // Two files: a defines WaterPort, b uses it via FeatureTyping.
        // The reverse-index entry keyed on WaterPort's id must include
        // a RelationshipTarget hit pointing at b.sysml's span — that's
        // the whole point of Phase 2 (find-references on the def site
        // must surface cross-file uses).
        let (host, pfs) = host_with_workspace(&[
            ("file:///a.sysml", "package Ports { port def WaterPort {} }"),
            (
                "file:///b.sysml",
                "package Uses { import Ports::*; part def Tank { port p : WaterPort; } }",
            ),
        ]);

        let idx = workspace_ref_index(host.analysis().db(), pfs);
        let map = idx.map();

        // Find WaterPort's element id (the def in a.sysml).
        let analysis = host.analysis();
        let elaborated = elaborate_workspace(analysis.db(), pfs);
        let water_port_id = elaborated
            .graph()
            .elements
            .values()
            .find(|e| e.name.as_deref() == Some("WaterPort") && e.kind.is_definition())
            .map(|e| e.id.clone())
            .expect("WaterPort def must exist in workspace");

        let sites = map
            .get(&water_port_id)
            .expect("WaterPort must have ref entries");

        let def_sites: Vec<_> = sites
            .iter()
            .filter(|s| matches!(s.kind, RefKind::Definition))
            .collect();
        let rel_sites: Vec<_> = sites
            .iter()
            .filter(|s| matches!(s.kind, RefKind::RelationshipTarget))
            .collect();

        assert!(!def_sites.is_empty(), "expected ≥1 Definition site, got {sites:?}");
        assert!(
            !rel_sites.is_empty(),
            "expected ≥1 cross-file RelationshipTarget site, got {sites:?}"
        );
        let in_b = rel_sites.iter().any(|s| s.file.ends_with("b.sysml"));
        assert!(
            in_b,
            "expected a RelationshipTarget hit in b.sysml; got {rel_sites:?}"
        );
    }

    #[test]
    fn workspace_ref_index_tags_library_provenance() {
        // Without a library loaded, every entry is Workspace.
        let (host, pfs) = host_with_workspace(&[("file:///test.sysml", "part def Widget;")]);
        let idx = workspace_ref_index(host.analysis().db(), pfs);
        for site in idx.map().values().flatten() {
            assert_eq!(
                site.provenance,
                Provenance::Workspace,
                "no-library variant must tag every site Workspace, got {site:?}"
            );
        }
    }
}
