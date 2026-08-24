//! Typed data for view families that do not render a node-edge scene.

use serde::Serialize;

use crate::gmodel::GeometryModel;
use crate::tmodel::TableModel;
use crate::tree::TreeModel;

/// The structured output for Grid, Geometry, and Browser views.
///
/// Consumers dispatch directly on this data when [`crate::ViewModel::non_graph`]
/// is present. It is renderer-neutral and deliberately has no graph-renderer
/// compatibility wrapper.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "lowercase")]
pub enum NonGraphModel {
    /// Tabular traceability data for `GridView`.
    Table(TableModel),
    /// Spatial primitives and viewport for `GeometryView`.
    Geometry(GeometryModel),
    /// Hierarchical containment data for `BrowserView`.
    Tree(TreeModel),
}

impl NonGraphModel {
    /// Wire-format discriminator (`"table"`, `"geometry"`, or `"tree"`).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Table(_) => "table",
            Self::Geometry(_) => "geometry",
            Self::Tree(_) => "tree",
        }
    }
}
