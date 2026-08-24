//! Diagram overlays — post-processors that transform a `DiagramIR`.
//!
//! Overlays let consumers opt into visual fidelity that the standard
//! generators don't carry on their own. They run after the generator
//! produces the IR and before render emits legacy graph, so they can:
//!
//! - Add CSS extras (e.g. `diagnostic-error` overlays)
//! - Add or restructure compartments
//! - Add or modify edge labels
//! - Add nodes pulled in via relationship traversal
//!
//! ## Why overlays exist
//!
//! Overlays are a generic post-processing substrate over `DiagramIR`.
//! The structural infrastructure here (the [`DiagramOverlay`] trait and
//! [`apply_all`]) is the foundation the service-layer `sim_overlay`
//! sidecar builds on. (Note: per-run / session-derived overlays like the
//! sim or verdict sidecars live at the service layer keyed by
//! `ElementId`, NOT as structural `DiagramIR` post-processors — see
//! `sim_overlay.rs` for that boundary.)
//!
//! ## Contract
//!
//! - **Idempotent**: applying the same overlay twice MUST produce the
//!   same output as applying it once. Overlays should check before they
//!   add state (e.g. don't push a CSS class that's already present).
//! - **Order-independent where practical**: overlays should not depend
//!   on each other's outputs. When ordering matters, document it.
//! - **Stateless**: overlays are unit structs without configurable
//!   state. Use multiple distinct overlays instead of one parameterised
//!   overlay.
//! - **Bounded cost**: overlays should be cheap (linear in IR size or
//!   graph size). Expensive computation (e.g. solver propagation) is
//!   acceptable but should be cached internally for one application.

use sysml_core::ModelGraph;

use crate::ir::types::DiagramIR;

/// A diagram overlay — a post-processor that mutates a `DiagramIR` in
/// place.
///
/// Implement on a unit struct, register it as a `&'static`, and add it
/// to a preset's overlay list. See module docs for the contract.
///
/// `Debug` is required so structs holding `&dyn DiagramOverlay`
/// (`ViewRequest`, `ViewPreset`) can derive `Debug` for tracing.
/// Implementations should print just the overlay name.
pub trait DiagramOverlay: std::fmt::Debug + Send + Sync {
    /// Stable identifier for this overlay (used for tracing and
    /// idempotency tracking). Should be `kebab-case` and unique within
    /// the diagram crate.
    fn name(&self) -> &'static str;

    /// Apply this overlay to the IR. Receives the source `ModelGraph`
    /// in case the overlay needs to traverse for elements/relationships
    /// that the generator didn't carry into the IR.
    fn apply(&self, ir: &mut DiagramIR, graph: &ModelGraph);
}

/// A list of overlays attached to a [`crate::ViewRequest`] or preset.
///
/// Stored as `&'static dyn DiagramOverlay` so transports can pass
/// overlays by reference without allocation. Preset registry entries
/// hold these as static slices.
pub type OverlayList = Vec<&'static dyn DiagramOverlay>;

/// Apply every overlay in `overlays` to `ir`, in order.
///
/// Logs each overlay's name at trace level so failures and unexpected
/// no-ops are visible in tracing output.
pub fn apply_all(overlays: &[&'static dyn DiagramOverlay], ir: &mut DiagramIR, graph: &ModelGraph) {
    for overlay in overlays {
        tracing::trace!(overlay = overlay.name(), "applying diagram overlay");
        overlay.apply(ir, graph);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::types::DiagramNode;
    use crate::ViewType;
    use crate::visual_kind::VisualKind;

    /// Test overlay: tags every top-level node with an `info` diagnostic.
    #[derive(Debug)]
    struct TaggingOverlay;
    impl DiagramOverlay for TaggingOverlay {
        fn name(&self) -> &'static str {
            "test-tagging"
        }
        fn apply(&self, ir: &mut DiagramIR, _graph: &ModelGraph) {
            for node in ir.nodes.iter_mut() {
                node.diagnostic_severity = Some("info".to_owned());
            }
        }
    }
    static TAGGING: TaggingOverlay = TaggingOverlay;

    fn make_ir_with_one_node() -> DiagramIR {
        let mut ir = DiagramIR::new(ViewType::General);
        ir.nodes.push(DiagramNode::new(
            "n1".to_owned(),
            VisualKind::Generic,
            "node".to_owned(),
        ));
        ir
    }

    #[test]
    fn apply_all_runs_overlays_in_order() {
        let graph = ModelGraph::new();
        let mut ir = make_ir_with_one_node();
        apply_all(&[&TAGGING], &mut ir, &graph);
        assert_eq!(ir.nodes[0].diagnostic_severity, Some("info".to_owned()));
    }

    #[test]
    fn overlay_is_idempotent() {
        let graph = ModelGraph::new();
        let mut ir = make_ir_with_one_node();
        apply_all(&[&TAGGING, &TAGGING, &TAGGING], &mut ir, &graph);
        // Tagging applied 3x; setting the severity is naturally idempotent.
        assert_eq!(ir.nodes[0].diagnostic_severity, Some("info".to_owned()));
    }

    #[test]
    fn empty_overlay_list_is_no_op() {
        let graph = ModelGraph::new();
        let mut ir = make_ir_with_one_node();
        let before = ir.nodes[0].diagnostic_severity.clone();
        apply_all(&[], &mut ir, &graph);
        assert_eq!(ir.nodes[0].diagnostic_severity, before);
    }
}
