//! Tagged diagram payload returned by the visualization pipeline.
//!
//! Different view families produce different shapes:
//! - Graph-shaped views (General, Interconnection, ActionFlow, StateTransition,
//!   Sequence, Requirements, Parametric) emit Sprotty `SGraph` JSON.
//! - GridView emits a `TableModel` (rows × columns).
//! - GeometryView emits a `GeometryModel` (spatial primitives + viewport).
//! - BrowserView emits a `TreeModel` (containment tree).
//!
//! `DiagramPayload` is the tagged union returned to consumers. Wire format:
//!
//! ```json
//! { "kind": "graph" | "table" | "geometry" | "tree", "data": ... }
//! ```
//!
//! ## Path-of-truth for non-graph views
//!
//! The REST endpoint (`GET /models/:uri/diagram`) and the MCP
//! `sysml_diagram` tool both serve the typed `DiagramPayload` via
//! [`crate::smodel::to_payload_with_filter_cache`]. Non-graph views (Grid / Geometry /
//! Browser) route to dedicated typed builders ([`crate::tmodel`] /
//! [`crate::gmodel`] / [`crate::tree`]) and never round-trip through
//! `SGraph`.
//!
//! The Sprotty generators for those three views still exist in
//! `crate::ir::generators::{grid, geometry, browser}` because the LSP
//! `sysml/diagram/setModel` push notification and the CLI
//! `export smodel --view <kind>` path expect raw `SGraph`. Those are
//! the only remaining callers; deleting the generators is queued for
//! the LSP/CLI migration to typed payloads.

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

use crate::gmodel::GeometryModel;
use crate::smodel::SGraph;
use crate::tmodel::TableModel;
use crate::tree::TreeModel;

/// The non-graph view families — the typed structured models the dedicated
/// renderers (TanStack Table / native tree / SVG-geometry) consume. Factored out
/// (3.12) so it has ONE home: both [`DiagramPayload`] (legacy SModel `/render`
/// path) and [`crate::ViewModel::non_graph`] (the single ViewModel pipeline)
/// reference it instead of each re-listing the three-way shape (principle #4/#5).
///
/// Wire format mirrors the old `DiagramPayload` tags exactly:
/// `{ "kind": "table" | "geometry" | "tree", "data": … }`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "lowercase")]
pub enum NonGraphModel {
    /// Tabular payload (rows × columns × cell types) — used by `GridView`.
    Table(TableModel),
    /// Spatial / floor-plan payload — used by `GeometryView`.
    Geometry(GeometryModel),
    /// Hierarchical containment tree — used by `BrowserView`.
    Tree(TreeModel),
}

impl NonGraphModel {
    /// Wire-format discriminator (`"table"` / `"geometry"` / `"tree"`).
    pub fn kind(&self) -> &'static str {
        match self {
            NonGraphModel::Table(_) => "table",
            NonGraphModel::Geometry(_) => "geometry",
            NonGraphModel::Tree(_) => "tree",
        }
    }
}

/// Tagged payload covering every diagram-family output: a graph-shaped `SGraph`
/// or one of the non-graph families. The wire format is FLAT —
/// `{ "kind": "graph"|"table"|"geometry"|"tree", "data": … }` — so the manual
/// `Serialize` delegates the non-graph arm to [`NonGraphModel`] (which already
/// serialises `{kind,data}`) rather than nesting it under a `NonGraph` wrapper.
#[derive(Debug, Clone)]
pub enum DiagramPayload {
    /// Sprotty `SGraph` for graph-shaped views.
    Graph(SGraph),
    /// One of the non-graph families (Table / Geometry / Tree).
    NonGraph(NonGraphModel),
}

impl DiagramPayload {
    /// Wire-format discriminator. Useful for callers that don't deserialize
    /// the full payload.
    pub fn kind(&self) -> &'static str {
        match self {
            DiagramPayload::Graph(_) => "graph",
            DiagramPayload::NonGraph(ng) => ng.kind(),
        }
    }
}

impl Serialize for DiagramPayload {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            // {kind:"graph", data:<SGraph>}
            DiagramPayload::Graph(g) => {
                let mut st = serializer.serialize_struct("DiagramPayload", 2)?;
                st.serialize_field("kind", "graph")?;
                st.serialize_field("data", g)?;
                st.end()
            }
            // NonGraphModel already serialises as {kind:"table"|…, data:…} — keep
            // the wire flat (no `NonGraph` wrapper) by delegating.
            DiagramPayload::NonGraph(ng) => ng.serialize(serializer),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_payload_serialises_with_kind_and_data() {
        let payload = DiagramPayload::Graph(SGraph::default());
        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value.get("kind").and_then(|v| v.as_str()), Some("graph"));
        assert!(value.get("data").is_some(), "payload must carry data field");
    }

    #[test]
    fn graph_payload_data_is_the_sgraph() {
        let mut sgraph = SGraph::default();
        sgraph.id = "root".to_owned();
        sgraph.type_ = "graph".to_owned();
        let payload = DiagramPayload::Graph(sgraph);
        let value = serde_json::to_value(&payload).unwrap();
        let data = value.get("data").unwrap();
        assert_eq!(data.get("id").and_then(|v| v.as_str()), Some("root"));
        assert_eq!(data.get("type").and_then(|v| v.as_str()), Some("graph"));
    }

    #[test]
    fn kind_discriminator_matches_serialisation() {
        let payload = DiagramPayload::Graph(SGraph::default());
        assert_eq!(payload.kind(), "graph");
    }

    #[test]
    fn non_graph_payload_serialises_flat_with_kind_and_data() {
        // The NonGraph arm must stay wire-compatible with the old flat tagging
        // ({kind:"table", data:…}) — no `NonGraph` wrapper key.
        let payload = DiagramPayload::NonGraph(NonGraphModel::Table(TableModel::default()));
        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value.get("kind").and_then(|v| v.as_str()), Some("table"));
        assert!(value.get("data").is_some());
        assert!(value.get("NonGraph").is_none(), "wire must stay flat, no wrapper");
        assert_eq!(payload.kind(), "table");
    }

    #[test]
    fn non_graph_model_serialises_same_as_payload_arm() {
        let ng = NonGraphModel::Tree(TreeModel::default());
        let direct = serde_json::to_value(&ng).unwrap();
        let via_payload = serde_json::to_value(DiagramPayload::NonGraph(ng)).unwrap();
        assert_eq!(direct, via_payload);
        assert_eq!(direct.get("kind").and_then(|v| v.as_str()), Some("tree"));
    }
}
