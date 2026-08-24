//! Renderer-neutral diagram notification shapes for the LSP.

#![allow(clippy::needless_pass_by_value)]

use sysml_diagram::ViewType;

/// Custom notification for a renderer-neutral ViewModel update.
pub(crate) const DIAGRAM_SET_VIEW_MODEL_METHOD: &str = "sysml/diagram/setViewModel";

/// Parameters for [`DIAGRAM_SET_VIEW_MODEL_METHOD`].
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagramSetViewModelParams {
    /// Document URI the view belongs to.
    pub uri: String,
    /// Canonical standard view-definition name that was rendered.
    pub view_type: String,
    /// Serialized [`sysml_diagram::ViewModel`].
    pub view_model: serde_json::Value,
}

/// Convert a view type to its canonical standard-library definition name.
pub(crate) fn view_type_name(view_type: ViewType) -> &'static str {
    sysml_service::diagram::view_type_name(view_type)
}

/// Parse a request view type through the service's transport mapping.
pub(crate) fn parse_view_type(value: &str) -> ViewType {
    sysml_service::diagram::parse_view_type(value)
}
