//! Visualization and diagram generation operations.
//!
//! Wraps `sysml-diagram` functionality for diagram generation and export.

use std::collections::HashSet;
use sysml_core::ModelGraph;
use sysml_diagram::smodel::{self, ViewType};
use sysml_diagram::ViewRequest;

/// Re-export ViewType for consumers.
pub use sysml_diagram::smodel::ViewType as DiagramViewType;

/// True for view types whose [`smodel::to_payload_with_filter_cache`] dispatch
/// goes through the heavy `to_smodel_with` SGraph path (vs the lighter
/// Tree / Table / Geometry payloads). Used by the cache router to
/// decide when [`crate::SysmlService::cached_smodel`] applies.
pub fn view_type_produces_sgraph(view_type: ViewType) -> bool {
    !matches!(
        view_type,
        ViewType::Grid | ViewType::Geometry | ViewType::Browser
    )
}

/// Wrap a cached SGraph in a tagged `DiagramPayload::Graph` and
/// serialise it to the canonical `{"kind":"graph","data":{…SGraph…}}` JSON shape.
pub fn graph_payload_json(sgraph: &smodel::SGraph) -> serde_json::Value {
    let payload = sysml_diagram::DiagramPayload::Graph(sgraph.clone());
    serde_json::to_value(&payload).expect(
        "DiagramPayload::Graph wraps an SGraph — both are internal Serde-derive and cannot fail to serialize",
    )
}

/// Build a `ViewRequest` for a user-authored `ViewUsage` /
/// `ViewDefinition` using a precomputed `ViewSummary` slice.
///
/// Looks `view_id` up in `summaries` (typically the salsa-cached
/// `Arc<Vec<ViewSummary>>` returned by
/// `SysmlService::workspace_view_index`), then composes a request via
/// [`ViewRequest::from_view_usage`]. Returns `None` if the id doesn't
/// point at a known view (caller can surface a 404). `expanded_ids`
/// from the transport are merged on top of any auto-expansion the
/// composer added (so the FE can keep its own expand/collapse state).
///
/// Per ADR-011 §3 / S3.T6a, prefer this signature on every call path
/// that has access to the cached summary list — the free-function
/// `build_view_request_for_view_usage` below is a back-compat shim
/// that re-derives the index from the graph each call.
pub fn build_view_request_for_view_usage_with_summaries(
    graph: &ModelGraph,
    summaries: &[sysml_core::ViewSummary],
    view_id: &sysml_core::ElementId,
    expanded_ids: &HashSet<String>,
) -> Option<ViewRequest> {
    let summary = summaries.iter().find(|s| s.id == *view_id)?;
    let mut req = ViewRequest::from_view_usage(graph, summary);
    for id in expanded_ids {
        req.expanded_ids.insert(id.clone());
    }
    Some(req)
}

/// Build a `ViewRequest` for a user-authored `ViewUsage` /
/// `ViewDefinition`.
///
/// Looks the view up in `graph` via [`sysml_core::build_view_index`],
/// then composes a request via [`ViewRequest::from_view_usage`].
/// Returns `None` if the id doesn't point at a known view (caller can
/// surface a 404). `expanded_ids` from the transport are merged on top
/// of any auto-expansion the composer added (so the FE can keep its
/// own expand/collapse state).
///
/// **Prefer [`build_view_request_for_view_usage_with_summaries`]**
/// on every path that has access to a `SysmlService` — the salsa
/// cache (`workspace_view_index`) amortises the graph walk across
/// calls within the same revision. This free function rebuilds the
/// index per call and is retained for callers without service
/// access (tests, ad-hoc tooling).
pub fn build_view_request_for_view_usage(
    graph: &ModelGraph,
    view_id: &sysml_core::ElementId,
    expanded_ids: &HashSet<String>,
) -> Option<ViewRequest> {
    let summaries = sysml_core::build_view_index(graph);
    build_view_request_for_view_usage_with_summaries(
        graph,
        &summaries,
        view_id,
        expanded_ids,
    )
}

/// Export a model graph as canonical JSON.
pub fn export_json(graph: &ModelGraph) -> String {
    sysml_core::json::to_json_string(graph)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::{Element, ElementKind};

    fn test_graph() -> ModelGraph {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPkg");
        let pkg_id = graph.add_element(pkg);
        let part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("engine")
            .with_owner(pkg_id);
        graph.add_element(part);
        graph
    }

    #[test]
    fn test_export_json() {
        let graph = test_graph();
        let json_str = export_json(&graph);
        // Verify it's valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(parsed.is_object());
    }
}
