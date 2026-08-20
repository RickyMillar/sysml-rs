//! Diagram notification wire shapes + tiny marshaling helpers for the LSP.
//!
//! The SModel projection (build SGraph, overlay diagnostics, prune stale
//! expanded ids) lives in `sysml-service` (`sysml_service::diagram`). All
//! transports share it. This module keeps only what's transport-specific:
//! LSP notification methods, their parameter types, and the
//! `sysml/diagram/setModelGraph` raw-ModelGraph serializer (a one-line
//! `serde_json::to_value` of the graph, transport-side because the LSP is
//! the only consumer of that wire today).

#![allow(clippy::needless_pass_by_value)]

use std::time::Instant;
use sysml_core::ModelGraph;
use sysml_diagram::smodel::ViewType;

/// Custom notification method for sending diagram models to the client.
pub(crate) const DIAGRAM_SET_MODEL_METHOD: &str = "sysml/diagram/setModel";

/// Custom notification method for sending the full ModelGraph JSON to the client.
/// The client can use this with WASM sysml-diagram to do local expand/collapse
/// without round-tripping through the LSP server.
pub(crate) const DIAGRAM_SET_MODEL_GRAPH_METHOD: &str = "sysml/diagram/setModelGraph";

/// Parameters for the sysml/diagram/setModel notification.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagramSetModelParams {
    /// The document URI this diagram is for
    pub uri: String,
    /// The view type name actually rendered.
    pub view_type: String,
    /// The SModel JSON (as a serde_json::Value for efficient serialization)
    pub model: serde_json::Value,
}

/// Parameters for the sysml/diagram/setModelGraph notification.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagramSetModelGraphParams {
    /// The document URI this ModelGraph is for
    pub uri: String,
    /// The full ModelGraph serialized as JSON
    pub model_graph: serde_json::Value,
    /// The current view type name
    pub view_type: String,
}

/// Convert a `ViewType` to its canonical wire name. Mirrors
/// `sysml_service::diagram::view_type_name` (kept here so the LSP can
/// stamp the view-type onto notification payloads without going through
/// the service crate boundary).
pub(crate) fn view_type_name(vt: ViewType) -> &'static str {
    sysml_service::diagram::view_type_name(vt)
}

/// Parse a view-type string from command arguments. Mirrors
/// `sysml_service::diagram::parse_view_type`.
pub(crate) fn parse_view_type(s: &str) -> ViewType {
    sysml_service::diagram::parse_view_type(s)
}

/// Build the `sysml/diagram/setModelGraph` notification payload — raw
/// ModelGraph JSON, used by the client-side WASM expand/collapse path.
#[tracing::instrument(level = "debug", skip(graph))]
pub(crate) fn build_set_model_graph_params(
    uri: &str,
    graph: &ModelGraph,
    view_type: ViewType,
) -> DiagramSetModelGraphParams {
    let start = Instant::now();
    let model_graph = serde_json::to_value(graph).unwrap_or_default();
    let json_size_bytes = serde_json::to_string(&model_graph).map(|s| s.len()).unwrap_or(0);
    tracing::info!(
        json_size_bytes,
        view_type = %view_type_name(view_type),
        uri = %uri,
        elapsed_ms = start.elapsed().as_millis() as u64,
        "model graph serialized for client WASM"
    );
    DiagramSetModelGraphParams {
        uri: uri.to_owned(),
        model_graph,
        view_type: view_type_name(view_type).to_owned(),
    }
}
