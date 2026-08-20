//! # sysml-vis
//!
//! Visualization exporters for SysML v2 ModelGraph.
//!
//! This crate provides:
//! - **ViewModel**: renderer-agnostic scene + tokens (the going-forward wire
//!   format for the React-SVG renderer)
//! - **SModel**: Sprotty-compatible diagram model (legacy SGraph format, still
//!   shipped by CLI/LSP/MCP/REST until they migrate to ViewModel)
//! - **PlantUML**: Text-based UML diagram export
//!
//! ## Example
//!
//! ```
//! use sysml_core::ModelGraph;
//! use sysml_diagram::{to_view_model, ViewRequest};
//! use sysml_diagram::smodel::ViewType;
//!
//! let graph = ModelGraph::new();
//! let request = ViewRequest::new(ViewType::General);
//! let view_model = to_view_model(&graph, &request);
//! ```

mod action_plantuml;
pub mod design_tokens;
pub mod diagnostic_overlay;
pub mod gmodel;
pub mod interaction;
pub mod ir;
pub mod payload;
mod plantuml;
pub mod sim_overlay;
pub mod smodel;
pub mod tmodel;
pub mod text_map;
pub mod tree;
pub mod verdict_overlay;
pub mod view_model;
pub mod view_request;
pub mod visual_kind;

pub use design_tokens::{
    CategoryColors, Color, DesignTokens, LinkColors, Palette, PortColors, SevColors, SimColors,
    VerdictColors,
};
pub use gmodel::{GeometryModel, GeometryPrimitive, Viewport};
pub use interaction::{build_interaction_map, InteractionEntry, InteractionMap};
pub use diagnostic_overlay::{
    build_diagnostic_overlay, DiagnosticItem, DiagnosticOverlay, ElementDiagnostics,
};
pub use payload::{DiagramPayload, NonGraphModel};
pub use sim_overlay::{
    build_sim_overlay, Activity, ElementOverlay, OverlayChannel, OverlayValue, SimOverlay,
};
pub use text_map::{build_text_map, TextMap, TextSpan};
pub use verdict_overlay::{
    build_verdict_overlay, ElementVerdict, VerdictOverlay, VerificationVerdicts,
};
pub use tmodel::{TableCell, TableColumn, TableColumnKind, TableModel, TableRow};
pub use tree::{TreeModel, TreeNode};
pub use view_model::{
    to_view_model, to_view_model_with_filter_cache, view_frame_from_summary, FrameSlot, ViewFrame,
    ViewModel,
};
pub use view_request::{DiagramRequestKey, ViewRequest};

pub use visual_kind::{
    ArrowHead, CompartmentKind, EdgeStyle, GraphicalKind, LineStyle, Shape, VisualKind,
};

pub use action_plantuml::{to_plantuml_activity, to_plantuml_sequence, SequenceEvent};
pub use plantuml::{to_plantuml, to_plantuml_state_view};

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_core::{Element, ElementKind, ModelGraph, Relationship, RelationshipKind};

    fn create_test_graph() -> ModelGraph {
        let mut graph = ModelGraph::new();

        // Package
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPackage");
        let pkg_id = graph.add_element(pkg);

        // Part
        let part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("Engine")
            .with_owner(pkg_id.clone());
        let part_id = graph.add_element(part);

        // Requirement
        let req = Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("SafetyReq")
            .with_owner(pkg_id);
        let req_id = graph.add_element(req);

        // Satisfy relationship
        let satisfy = Relationship::new(RelationshipKind::Satisfy, part_id, req_id);
        graph.add_relationship(satisfy);

        graph
    }

    #[test]
    fn plantuml_output_structure() {
        let graph = create_test_graph();
        let puml = to_plantuml(&graph);

        assert!(puml.starts_with("@startuml"));
        assert!(puml.ends_with("@enduml\n"));
    }

    #[test]
    fn plantuml_contains_elements() {
        let graph = create_test_graph();
        let puml = to_plantuml(&graph);

        assert!(puml.contains("TestPackage"));
        assert!(puml.contains("Engine"));
        assert!(puml.contains("SafetyReq"));
    }

    #[test]
    fn empty_graph() {
        let graph = ModelGraph::new();

        let puml = to_plantuml(&graph);
        assert!(puml.contains("@startuml"));
    }
}
