//! Static **diagnostics** overlay-delta — the diagram sidecar for validation
//! diagnostics, joined to a scene by `ElementId`.
//!
//! A renderer-agnostic description of which scene elements carry diagnostics
//! (a badge severity + tooltip detail). It is the diagnostics companion to
//! [`crate::sim_overlay`] (per-tick) and [`crate::verdict_overlay`] (per-run):
//! same identity-first join, same sparse/scene-scoped discipline.
//!
//! ## Why this is a service-layer sidecar, not a salsa `ViewModel` field
//!
//! Unlike `tokens` / `text_map` (pure functions of the graph, salsa-cached),
//! diagnostics are gated by the service's **readiness** state
//! (`SysmlService::library_lifecycle`) — a transient instance-level override
//! that is *not* a salsa input (steward ruling, 2026-07-14). So the artifact
//! cannot ride a graph-keyed salsa sidecar without promoting readiness to a
//! salsa input. It is therefore built at the **service layer** — like
//! `sim_overlay`/`verdict_overlay` — from already-memoized ingredients
//! (`self.diagnostics()`, itself readiness-gated) and delivered via
//! `sysml.diagram.diagnostic_overlay`. There is deliberately **no `overlays`
//! field on `ViewModel`** and this is not a structural [`crate::ir::overlays`]
//! post-processor (that trait is graph-only and stateless; it cannot see the
//! readiness gate).
//!
//! ## The span → `ElementId` join lives at the service layer
//!
//! A [`sysml_span::Diagnostic`] carries a **span**, never an `ElementId`. The
//! service resolves each diagnostic's span to an element (via the ide-db
//! `PositionMap::element_at` reverse index) and hands this builder plain
//! `(ElementId, &Diagnostic)` pairs. This module never depends on ide-db and
//! never parses a name — it joins by the hard `ElementId` only. A diagnostic
//! whose span resolves to no element, or to an element **not present in this
//! scene**, is skipped: sparse, scene-scoped, no fabricated placement (mirrors
//! [`crate::verdict_overlay`]).

use std::collections::{HashMap, HashSet};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use sysml_core::ElementId;
use sysml_span::{Diagnostic, Severity};

use crate::ir::types::{DiagramChild, DiagramIR, DiagramNode};

/// The diagnostics overlay for a diagram scene.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DiagnosticOverlay {
    /// Per-element diagnostics. **Sparse** — only scene elements that carry at
    /// least one joinable diagnostic.
    ///
    /// Key = [`ElementId::to_string`](sysml_core::ElementId::to_string) (the
    /// same string used as the scene node / compartment-row id, so the renderer
    /// joins directly to `DiagramNode::element_id`). **Never a name string** —
    /// every key derives from an `ElementId`.
    pub elements: HashMap<String, ElementDiagnostics>,
}

/// One element's diagnostics for this scene.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ElementDiagnostics {
    /// Worst-case severity across `items` — drives the badge colour. `Severity`
    /// is ordered `Info < Warning < Error`, serialized lowercase
    /// (`"info"`/`"warning"`/`"error"`).
    pub severity: Severity,
    /// Every diagnostic on this element, for the badge tooltip. At least one
    /// (empty entries are never emitted).
    pub items: Vec<DiagnosticItem>,
}

/// A single diagnostic attached to an element (tooltip detail).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DiagnosticItem {
    /// This diagnostic's own severity.
    pub severity: Severity,
    /// The diagnostic message.
    pub message: String,
    /// The error/warning code, when the diagnostic carries one.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub code: Option<String>,
}

/// Build the [`DiagnosticOverlay`] for `scene` from diagnostics the service has
/// already resolved to elements.
///
/// Pure and deterministic. `resolved` is the set of `(ElementId, &Diagnostic)`
/// pairs the service produced by resolving each diagnostic's span (the builder
/// does no span work — that needs the ide-db position index, which lives at the
/// service layer). Only pairs whose `ElementId` is present in `scene` (as a
/// node **or** a collapsed compartment row) contribute an entry; everything
/// else is skipped so the overlay stays sparse and scene-scoped.
pub fn build_diagnostic_overlay(
    scene: &DiagramIR,
    resolved: &[(ElementId, &Diagnostic)],
) -> DiagnosticOverlay {
    // Collect every ElementId the scene actually renders (nodes, nested nodes,
    // ports, and collapsed compartment `Text` rows) once, so the join is a
    // hard-id membership test — identity-first, never name-based.
    let scene_ids = collect_scene_ids(scene);

    let mut elements: HashMap<String, ElementDiagnostics> = HashMap::new();
    for (element_id, diag) in resolved {
        let key = element_id.to_string();
        if !scene_ids.contains(&key) {
            // Off-scene (or unresolved) — nothing to badge here. Skip; never
            // fabricate a placement.
            continue;
        }
        let entry = elements.entry(key).or_insert(ElementDiagnostics {
            severity: Severity::Info,
            items: Vec::new(),
        });
        entry.severity = entry.severity.max(diag.severity);
        entry.items.push(DiagnosticItem {
            severity: diag.severity,
            message: diag.message.clone(),
            code: diag.code.clone(),
        });
    }

    DiagnosticOverlay { elements }
}

/// Gather every `ElementId` string rendered anywhere in `scene`.
fn collect_scene_ids(scene: &DiagramIR) -> HashSet<String> {
    let mut ids = HashSet::new();
    for node in &scene.nodes {
        collect_node_ids(node, &mut ids);
    }
    ids
}

fn collect_node_ids(node: &DiagramNode, ids: &mut HashSet<String>) {
    ids.insert(node.element_id.clone());
    for port in &node.ports {
        ids.insert(port.element_id.clone());
    }
    for child in &node.children {
        collect_child_ids(child, ids);
    }
}

fn collect_child_ids(child: &DiagramChild, ids: &mut HashSet<String>) {
    match child {
        DiagramChild::Node(n) => collect_node_ids(n, ids),
        DiagramChild::Compartment { children, .. } => {
            for c in children {
                collect_child_ids(c, ids);
            }
        }
        DiagramChild::Island { subtree, .. } => {
            for n in &subtree.nodes {
                collect_node_ids(n, ids);
            }
        }
        // A collapsed compartment row (e.g. an attribute) carries its real
        // ElementId — a valid badge target, so include it.
        DiagramChild::Text { element_id, .. } => {
            ids.insert(element_id.clone());
        }
        DiagramChild::Edge(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ViewType;
    use crate::visual_kind::{CompartmentKind, VisualKind};

    fn eid(id: &str) -> String {
        ElementId::from_string(id).to_string()
    }

    fn node(id: &str, name: &str) -> DiagramNode {
        DiagramNode::new(eid(id), VisualKind::Part, name)
    }

    fn scene(nodes: Vec<DiagramNode>) -> DiagramIR {
        DiagramIR {
            view_type: ViewType::General,
            nodes,
            edges: Vec::new(),
            buttons: Vec::new(),
        }
    }

    /// A diagnostic resolved to a scene node badges that node; worst-case
    /// severity is reported and every message is kept for the tooltip.
    #[test]
    fn badges_scene_node_and_folds_worst_severity() {
        let scene = scene(vec![node("p-1", "Circuit")]);
        let p = ElementId::from_string(&eid("p-1"));
        let warn = Diagnostic::warning("suspicious unit").with_code("PH001");
        let err = Diagnostic::error("type mismatch").with_code("S002");
        let resolved = [(p.clone(), &warn), (p.clone(), &err)];

        let overlay = build_diagnostic_overlay(&scene, &resolved);
        let entry = &overlay.elements[&eid("p-1")];
        // Worst-case across the two → Error drives the badge.
        assert_eq!(entry.severity, Severity::Error);
        assert_eq!(entry.items.len(), 2);
        assert!(entry.items.iter().any(|i| i.message == "type mismatch"
            && i.code.as_deref() == Some("S002")));
    }

    /// A diagnostic on a collapsed attribute **compartment row** badges the row
    /// (joined by the row's real ElementId — leaning on hard ids).
    #[test]
    fn badges_collapsed_compartment_row() {
        let mut part = node("p-1", "Circuit");
        part.children.push(DiagramChild::Text {
            compartment: CompartmentKind::Attributes,
            text: "voltage : Real".to_owned(),
            element_id: eid("attr-v"),
            source: crate::ir::types::CompartmentItemSource::Owned,
        });
        let scene = scene(vec![part]);
        let a = ElementId::from_string(&eid("attr-v"));
        let d = Diagnostic::error("unresolved type");
        let resolved = [(a, &d)];

        let overlay = build_diagnostic_overlay(&scene, &resolved);
        assert!(overlay.elements.contains_key(&eid("attr-v")));
        assert_eq!(overlay.elements[&eid("attr-v")].severity, Severity::Error);
    }

    /// A diagnostic resolved to an element that is **not in this scene** is
    /// skipped — sparse and scene-scoped, no fabricated placement.
    #[test]
    fn skips_off_scene_element() {
        let scene = scene(vec![node("p-1", "Circuit")]);
        let ghost = ElementId::from_string(&eid("not-here"));
        let d = Diagnostic::error("boom");
        let resolved = [(ghost, &d)];

        let overlay = build_diagnostic_overlay(&scene, &resolved);
        assert!(overlay.elements.is_empty());
    }
}
