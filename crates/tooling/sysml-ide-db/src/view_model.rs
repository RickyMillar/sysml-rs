//! Tracked queries for `sysml_diagram::to_view_model` (Bucket 1.4).
//!
//! The [`ViewModel`] is the renderer-agnostic wire artifact a frontend consumes
//! (the promoted `DiagramIR` scene plus the addenda later Bucket-1 tasks attach).
//! Building it is a pure function of `(graph, request)` — it walks the graph,
//! runs the `ViewType`-specific generator, and applies overlays. No session state
//! is involved, so it is salsa-cacheable, exactly like the ViewModel query in
//! [`crate::diagram`].
//!
//! The cache key is the [`DiagramRequestKey`] newtype from `sysml-diagram` — the
//! same key the ViewModel query uses, restricting the key to
//! `(view_type, expanded_ids, expose)`. Requests carrying `filter` / `hints` /
//! `overlays` deliberately bypass the cache (see `crate::diagram` for the
//! rationale).
//!
//! Three variants mirror [`crate::diagram`] and every other tracked-query module:
//!
//! - `file_*` — single-file (no workspace, no library overlay).
//! - `workspace_*` — workspace-merged (no library overlay).
//! - `workspace_*_with_library` — workspace-merged with library overlay.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_core::{ModelGraph, ViewSummary};
use sysml_diagram::{to_view_model, view_frame_from_summary, DiagramRequestKey, ViewFrame, ViewModel};

use crate::analysis::{elaborate_workspace, elaborate_workspace_with_library};
use crate::parse;
use crate::project_inputs::ProjectFileSet;
use crate::resolution::LibraryGraph;
use crate::source::SourceFile;
use crate::interaction::{
    file_interaction_map, workspace_interaction_map, workspace_interaction_map_with_library,
};
use crate::text_map::{file_text_map, workspace_text_map, workspace_text_map_with_library};
use crate::view_index::{
    file_view_index, workspace_view_index, workspace_view_index_with_library,
};
use crate::Db;

/// Resolve the [`ViewFrame`] (§F-10) for a request key against the declared-view
/// index. A diagram is a framed-view only when it renders a declared `View`.
///
/// Identity is the declared view's own id: when the key carries `view_id`
/// (set by `ViewRequest::from_view_usage`) we match the `ViewSummary` by id
/// exactly, so two views sharing a kind + expose target each resolve their
/// own frame. The legacy expose-set match is the fallback for keys with no
/// `view_id` (ad-hoc/LSP projections). `None` when neither resolves — that
/// is not a framed-view.
fn frame_for_key(
    graph: &ModelGraph,
    summaries: &[ViewSummary],
    key: &DiagramRequestKey,
) -> Option<ViewFrame> {
    if let Some(view_id) = key.view_id() {
        let summary = summaries.iter().find(|s| &s.id == view_id)?;
        return Some(view_frame_from_summary(graph, summary, key.view_type()));
    }
    let expose = key.exposes().iter().next()?;
    let summary = summaries.iter().find(|s| {
        s.exposed
            .iter()
            .any(|e| e.exposed_element_id.as_ref() == Some(expose))
    })?;
    Some(view_frame_from_summary(graph, summary, key.view_type()))
}

/// Salsa-cached [`ViewModel`] produced by `to_view_model` for a stable
/// [`DiagramRequestKey`] against an unchanged elaborated graph.
///
/// Wraps `Arc<ViewModel>` with pointer-identity equality. `ViewModel` is a deep
/// tree of `DiagramIR` enum variants and does not implement `Eq`/`Hash` (hashing
/// the whole structure each call would defeat caching). Identity equality is
/// sufficient because the value is only compared within the same salsa revision.
#[derive(Clone, Debug)]
pub struct CachedViewModel(Arc<ViewModel>);

impl CachedViewModel {
    fn new(view_model: ViewModel) -> Self {
        Self(Arc::new(view_model))
    }

    /// Borrow the inner [`ViewModel`].
    pub fn view_model(&self) -> &ViewModel {
        &self.0
    }

    /// Clone the inner `Arc<ViewModel>` (cheap pointer bump).
    pub fn arc(&self) -> Arc<ViewModel> {
        Arc::clone(&self.0)
    }
}

salsa_arc_wrapper!(identity, CachedViewModel, ViewModel);

// ---------------------------------------------------------------------------
// to_view_model — three variants + best-shape dispatcher
// ---------------------------------------------------------------------------

/// Build a [`ViewModel`] for a single-file model graph.
///
/// Depends on: `parse_file()` (Layer 1).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_view_model(
    db: &dyn Db,
    source_file: SourceFile,
    key: DiagramRequestKey,
) -> CachedViewModel {
    let parsed = parse::parse_file(db, source_file);
    let request = key.to_view_request();
    let text_map = file_text_map(db, source_file);
    let interactions = file_interaction_map(db, source_file);
    let view_index = file_view_index(db, source_file);
    let frame = frame_for_key(parsed.graph(), view_index.summaries(), &key);
    CachedViewModel::new(
        to_view_model(parsed.graph(), &request)
            .with_text_map(text_map.arc())
            .with_interactions(interactions.arc())
            .with_frame(frame),
    )
}

/// Build a [`ViewModel`] for a workspace-merged graph (no library overlay).
///
/// Depends on: `elaborate_workspace()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_view_model(
    db: &dyn Db,
    pfs: ProjectFileSet,
    key: DiagramRequestKey,
) -> CachedViewModel {
    let elaborated = elaborate_workspace(db, pfs);
    let request = key.to_view_request();
    let text_map = workspace_text_map(db, pfs);
    let interactions = workspace_interaction_map(db, pfs);
    let view_index = workspace_view_index(db, pfs);
    let frame = frame_for_key(elaborated.graph(), view_index.summaries(), &key);
    CachedViewModel::new(
        to_view_model(elaborated.graph(), &request)
            .with_text_map(text_map.arc())
            .with_interactions(interactions.arc())
            .with_frame(frame),
    )
}

/// Build a [`ViewModel`] for a workspace-merged graph with library overlay.
///
/// Depends on: `elaborate_workspace_with_library()` (Layer 4).
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn workspace_view_model_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
    key: DiagramRequestKey,
) -> CachedViewModel {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    let request = key.to_view_request();
    let text_map = workspace_text_map_with_library(db, pfs, library);
    let interactions = workspace_interaction_map_with_library(db, pfs, library);
    let view_index = workspace_view_index_with_library(db, pfs, library);
    let frame = frame_for_key(elaborated.graph(), view_index.summaries(), &key);
    CachedViewModel::new(
        to_view_model(elaborated.graph(), &request)
            .with_text_map(text_map.arc())
            .with_interactions(interactions.arc())
            .with_frame(frame),
    )
}

/// Best-shape dispatcher — `Some(lib)` routes to `..._with_library`, `None` to
/// the bare workspace query.
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_view_model_best(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: Option<LibraryGraph>,
    key: DiagramRequestKey,
) -> CachedViewModel {
    match library {
        Some(lib) => workspace_view_model_with_library(db, pfs, lib, key),
        None => workspace_view_model(db, pfs, key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::AnalysisHost;
    use sysml_diagram::ViewType;
    use sysml_diagram::ViewRequest;

    fn key_for(vt: ViewType) -> DiagramRequestKey {
        ViewRequest::new(vt).cache_key().expect("plain request is cacheable")
    }

    #[test]
    fn file_view_model_caches_across_calls() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def Engine; }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");
        let key = key_for(ViewType::General);

        let analysis = host.analysis();
        let r1 = file_view_model(analysis.db(), sf, key.clone());
        let r2 = file_view_model(analysis.db(), sf, key);

        assert!(Arc::ptr_eq(&r1.0, &r2.0));
    }

    #[test]
    fn file_view_model_invalidates_on_content_change() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def A; }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");
        let key = key_for(ViewType::General);
        let r1 = file_view_model(host.analysis().db(), sf, key.clone()).arc();

        host.set_file_content(
            "test.sysml",
            "package P { part def B; }".to_string(),
        );
        let sf2 = host.source_file(id).expect("source file still exists");
        let r2 = file_view_model(host.analysis().db(), sf2, key).arc();

        assert!(!Arc::ptr_eq(&r1, &r2));
    }

    #[test]
    fn file_view_model_differs_by_view_type() {
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def Engine; }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let general = file_view_model(host.analysis().db(), sf, key_for(ViewType::General));
        let interconnection = file_view_model(
            host.analysis().db(),
            sf,
            key_for(ViewType::Interconnection),
        );

        // Distinct cache slots → distinct Arcs.
        assert!(!Arc::ptr_eq(&general.0, &interconnection.0));
    }

    #[test]
    fn view_model_scene_matches_diagram_query_structure() {
        // The ViewModel scene and the ViewModel query are derived from the same
        // generate path; the scene should carry the model's node(s).
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def Engine; part def Wheel; }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let vm = file_view_model(host.analysis().db(), sf, key_for(ViewType::General));
        assert!(
            !vm.view_model().scene().nodes.is_empty(),
            "scene should contain the model's nodes"
        );
    }

    #[test]
    fn view_model_carries_joinable_text_map() {
        // The text-map (1.6) is attached, and a scene node's element_id joins
        // against it to recover the source span (the diagram→text link).
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def Engine; }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let cached = file_view_model(host.analysis().db(), sf, key_for(ViewType::General));
        let vm = cached.view_model();
        let text_map = vm.text_map().expect("salsa layer attaches the text-map");
        assert!(!text_map.is_empty(), "text-map should map the model's elements");

        // Every real-element node id should resolve to a span.
        let joined = vm
            .scene()
            .nodes
            .iter()
            .any(|n| text_map.span_for(&n.element_id).is_some());
        assert!(joined, "a scene node id should join against the text-map");
    }

    #[test]
    fn frame_matches_declared_view_by_expose_and_is_none_for_ad_hoc() {
        use sysml_core::view_index::ExposeRef;
        use sysml_core::{ElementId, ElementKind};

        let engine = ElementId::from_string("engine");
        let summaries = vec![ViewSummary {
            id: ElementId::from_string("view-1"),
            name: Some("EngineView".to_owned()),
            kind: ElementKind::ViewUsage,
            exposed: vec![ExposeRef {
                id: ElementId::from_string("ex-1"),
                is_namespace: false,
                qualified_name: Some("Engine".to_owned()),
                exposed_element_id: Some(engine.clone()),
            }],
            renderings: Vec::new(),
            filters: Vec::new(),
            source_span: None,
        }];

        // A declared-view request (expose set) → frame resolved by expose match.
        let declared = ViewRequest::new(ViewType::General)
            .with_expose(engine)
            .cache_key()
            .expect("plain expose request is cacheable");
        let graph = ModelGraph::new();
        let frame = frame_for_key(&graph, &summaries, &declared).expect("declared view → frame");
        assert_eq!(frame.name, "EngineView");
        assert_eq!(frame.view_kind, ViewType::General);

        // An ad-hoc projection (no expose) → no frame.
        let ad_hoc = key_for(ViewType::General);
        assert!(frame_for_key(&graph, &summaries, &ad_hoc).is_none());
    }

    #[test]
    fn frame_resolves_by_view_id_when_views_share_an_expose() {
        use sysml_core::view_index::ExposeRef;
        use sysml_core::{ElementId, ElementKind};

        // Two declared views, same kind + same expose target — distinct
        // frames must resolve by the view's own id, not by the first
        // expose match (the WI1b aliasing bug).
        let shared = ElementId::from_string("Vehicle");
        let mk = |id: &str, name: &str| ViewSummary {
            id: ElementId::from_string(id),
            name: Some(name.to_owned()),
            kind: ElementKind::ViewDefinition,
            exposed: vec![ExposeRef {
                id: ElementId::from_string(format!("ex-{id}")),
                is_namespace: false,
                qualified_name: Some("Vehicle".to_owned()),
                exposed_element_id: Some(shared.clone()),
            }],
            renderings: Vec::new(),
            filters: Vec::new(),
            source_span: None,
        };
        let summaries = vec![mk("view-overview", "OverviewView"), mk("view-rendered", "RenderedView")];

        let mut req = ViewRequest::new(ViewType::General).with_expose(shared.clone());
        req.view_id = Some(ElementId::from_string("view-rendered"));
        let key = req.cache_key().expect("cacheable");
        let graph = ModelGraph::new();
        let frame = frame_for_key(&graph, &summaries, &key).expect("frame by view_id");
        assert_eq!(frame.name, "RenderedView", "must resolve its OWN frame, not the first expose match");
    }

    #[test]
    fn view_model_carries_interaction_map() {
        // The interaction map (1.5) is attached, and a typed usage's go-to-
        // definition target is recoverable by joining a scene node id.
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(
            "test.sysml",
            "package P { part def Engine; part p : Engine; }".to_string(),
        );
        let sf = host.source_file(id).expect("source file exists");

        let cached = file_view_model(host.analysis().db(), sf, key_for(ViewType::General));
        let vm = cached.view_model();
        let interactions = vm
            .interactions()
            .expect("salsa layer attaches the interaction map");
        assert!(
            !interactions.is_empty(),
            "the typed usage `p : Engine` should yield a go-to-definition affordance"
        );
    }
}
