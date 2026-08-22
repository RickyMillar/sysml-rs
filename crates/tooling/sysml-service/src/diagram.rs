//! Diagram SModel projection helpers for service-level commands.
//!
//! Hosts the SModel build + diagnostic-overlay primitive that powers
//! `sysml.diagram.{open,view,expand}` and `sysml.export.smodel`. Moved
//! out of `sysml-lsp-server/src/diagram.rs` per transport-bypass-bucket-b §B1
//! so all transports share one projection.
//!
//! Inputs are `sysml_span::Diagnostic` (the service-native shape). The LSP
//! used to convert its `LspDiagnostic` payload through a shim before
//! overlaying; with the projection on the service, diagnostics flow straight
//! through `service.diagnostics(uri)`.
//!
//! Public surface:
//! - [`parse_view_type`] — string → `ViewType` mapping shared with the LSP.
//! - [`view_type_name`] — canonical wire name for a `ViewType`.
//! - [`overlay_diagnostics`] — apply line-range severity overlays onto an
//!   `SGraph` in place.
//! - [`prune_expanded_ids`] — drop stale expanded ids whose elements no
//!   longer exist in the elaborated graph.

use std::collections::HashSet;

use sysml_core::ModelGraph;
use sysml_diagram::smodel::{
    types::{LabelDiagnostic, SGraph, SModelElement},
    ViewType,
};
use sysml_span::{Diagnostic, Severity};

/// Convert a `ViewType` to its canonical wire-name string.
pub fn view_type_name(vt: ViewType) -> &'static str {
    match vt {
        ViewType::General => "GeneralView",
        ViewType::Interconnection => "InterconnectionView",
        ViewType::StateTransition => "StateTransitionView",
        ViewType::ActionFlow => "ActionFlowView",
        ViewType::Browser => "BrowserView",
        ViewType::Sequence => "SequenceView",
        ViewType::Grid => "GridView",
        ViewType::Geometry => "GeometryView",
    }
}

/// Parse a view-type string from command arguments. Defaults to
/// `ViewType::General` for unrecognised inputs.
///
/// Thin wrapper over [`ViewType::from_request_str`] — the tolerant
/// wire-parameter mapping. Request deserialisation only; model/graph
/// resolution never goes through here.
pub fn parse_view_type(s: &str) -> ViewType {
    ViewType::from_request_str(s).unwrap_or(ViewType::General)
}

/// Drop expanded ids whose elements no longer exist in `graph`.
pub fn prune_expanded_ids(expanded: &mut HashSet<String>, graph: &ModelGraph) {
    expanded.retain(|id| {
        let eid = sysml_id::ElementId::from_string(id.as_str());
        graph.get_element(&eid).is_some()
    });
}

fn severity_rank(s: &Severity) -> u8 {
    match s {
        Severity::Info => 0,
        Severity::Warning => 1,
        Severity::Error => 2,
    }
}

/// Overlay diagnostic severity onto SModel nodes by matching source ranges.
///
/// For each node with a `source_range`, find the worst-severity diagnostic
/// whose `span.line` falls within `[start_line, end_line]` and stamp it
/// onto the node (`diagnostic_severity` + CSS class) and onto the inner
/// `label:name` (including labels nested inside a `comp:header` compartment).
///
/// Returns the number of nodes that received a diagnostic overlay.
pub fn overlay_diagnostics(sgraph: &mut SGraph, diagnostics: &[Diagnostic]) -> usize {
    struct DiagEntry {
        line: u32,
        severity: Severity,
        message: String,
        code: Option<String>,
        tags: Vec<String>,
    }

    let entries: Vec<DiagEntry> = diagnostics
        .iter()
        .filter_map(|d| {
            d.span.as_ref().and_then(|s| {
                s.line.map(|line| DiagEntry {
                    line,
                    severity: d.severity,
                    message: d.message.clone(),
                    code: d.code.clone(),
                    tags: d
                        .tags
                        .iter()
                        .map(|t| match t {
                            sysml_span::DiagnosticTag::Unnecessary => "unnecessary".to_owned(),
                            sysml_span::DiagnosticTag::Deprecated => "deprecated".to_owned(),
                        })
                        .collect(),
                })
            })
        })
        .collect();

    if entries.is_empty() {
        return 0;
    }

    fn find_worst<'a>(
        entries: &'a [DiagEntry],
        start_line: u32,
        end_line: u32,
    ) -> Option<&'a DiagEntry> {
        let mut worst: Option<&DiagEntry> = None;
        for entry in entries {
            if entry.line >= start_line && entry.line <= end_line {
                worst = Some(match worst {
                    None => entry,
                    Some(prev) => {
                        if severity_rank(&entry.severity) > severity_rank(&prev.severity) {
                            entry
                        } else {
                            prev
                        }
                    }
                });
            }
        }
        worst
    }

    fn walk(elements: &mut [SModelElement], entries: &[DiagEntry]) -> usize {
        let mut count = 0;
        for el in elements {
            match el {
                SModelElement::Node(node) => {
                    if let Some(source_range) = &node.source_range {
                        let start_line = source_range[0];
                        let end_line = source_range[2];
                        if let Some(worst) = find_worst(entries, start_line, end_line) {
                            let class = match worst.severity {
                                Severity::Error => "diagnostic-error",
                                Severity::Warning => "diagnostic-warning",
                                Severity::Info => "diagnostic-info",
                            };
                            node.diagnostic_severity =
                                Some(class.replace("diagnostic-", ""));
                            if !node.css_classes.iter().any(|c| c == class) {
                                node.css_classes.push(class.to_owned());
                            }

                            let quick_fixes: Vec<String> = match worst.code.as_deref() {
                                Some("E200") => vec![
                                    "Auto-import".into(),
                                    "Use qualified name".into(),
                                    "Create definition".into(),
                                ],
                                Some("S001") => vec!["Rename duplicate".into()],
                                _ if worst.message.contains("expected")
                                    && worst.message.contains(';') =>
                                {
                                    vec!["Insert semicolon".into()]
                                }
                                _ if worst.message.contains("expected")
                                    && worst.message.contains('}') =>
                                {
                                    vec!["Insert closing brace".into()]
                                }
                                _ => vec![],
                            };

                            let label_diag = LabelDiagnostic {
                                severity: node
                                    .diagnostic_severity
                                    .clone()
                                    .unwrap_or_default(),
                                message: worst.message.clone(),
                                code: worst.code.clone(),
                                tags: worst.tags.clone(),
                                quick_fixes,
                            };
                            for child in &mut node.children {
                                if let SModelElement::Label(label) = child {
                                    if label.type_ == "label:name" {
                                        label.diagnostic = Some(label_diag.clone());
                                    }
                                }
                                if let SModelElement::Compartment(comp) = child {
                                    for gc in &mut comp.children {
                                        if let SModelElement::Label(label) = gc {
                                            if label.type_ == "label:name" {
                                                label.diagnostic = Some(label_diag.clone());
                                            }
                                        }
                                    }
                                }
                            }
                            count += 1;
                        }
                    }
                    count += walk(&mut node.children, entries);
                }
                SModelElement::Compartment(comp) => {
                    count += walk(&mut comp.children, entries);
                }
                _ => {}
            }
        }
        count
    }

    walk(&mut sgraph.children, &entries)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::{Element, ElementKind};
    use sysml_diagram::smodel::types::{SCompartment, SLabel, SNode};
    use sysml_id::ElementId;

    #[test]
    fn parses_known_view_types() {
        assert_eq!(parse_view_type("general"), ViewType::General);
        assert_eq!(parse_view_type("StateTransitionView"), ViewType::StateTransition);
        assert_eq!(parse_view_type("unknown"), ViewType::General);
    }

    #[test]
    fn view_type_names_are_camel() {
        assert_eq!(view_type_name(ViewType::General), "GeneralView");
        assert_eq!(view_type_name(ViewType::StateTransition), "StateTransitionView");
    }

    #[test]
    fn prune_expanded_ids_drops_stale() {
        let mut graph = ModelGraph::new();
        let id_a = ElementId::from_string("elem-a");
        graph.add_element(Element::new(id_a.clone(), ElementKind::PartUsage));

        let mut expanded: HashSet<String> = HashSet::new();
        expanded.insert(id_a.to_string());
        expanded.insert("stale-id".to_string());

        prune_expanded_ids(&mut expanded, &graph);
        assert_eq!(expanded.len(), 1);
        assert!(expanded.contains(&id_a.to_string()));
    }

    #[test]
    fn overlay_propagates_to_label_name() {
        let mut sgraph = SGraph {
            id: "graph".into(),
            type_: "graph".into(),
            children: vec![SModelElement::Node(SNode {
                id: "node-1".into(),
                type_: "node:block".into(),
                source_range: Some([10, 0, 15, 0]),
                children: vec![SModelElement::Label(SLabel {
                    id: "label-1".into(),
                    type_: "label:name".into(),
                    text: "X".into(),
                    ..Default::default()
                })],
                ..Default::default()
            })],
            ..Default::default()
        };

        let diags = vec![Diagnostic {
            severity: Severity::Error,
            code: Some("E200".into()),
            message: "unresolved".into(),
            span: Some(sysml_span::Span {
                file: "t.sysml".into(),
                start: 0,
                end: 1,
                line: Some(12),
                col: Some(1),
            }),
            notes: vec![],
            related: vec![],
            tags: vec![],
            tier: sysml_span::DiagnosticTier::default(),
        }];

        assert_eq!(overlay_diagnostics(&mut sgraph, &diags), 1);

        let node = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected node"),
        };
        assert_eq!(node.diagnostic_severity.as_deref(), Some("error"));
        let label = match &node.children[0] {
            SModelElement::Label(l) => l,
            _ => panic!("expected label"),
        };
        assert!(label.diagnostic.is_some());
    }

    #[test]
    fn overlay_handles_nested_compartment() {
        let mut sgraph = SGraph {
            id: "graph".into(),
            type_: "graph".into(),
            children: vec![SModelElement::Node(SNode {
                id: "node-1".into(),
                type_: "node:package".into(),
                source_range: Some([1, 0, 20, 0]),
                children: vec![SModelElement::Compartment(SCompartment {
                    id: "comp-header".into(),
                    type_: "comp:header".into(),
                    children: vec![SModelElement::Label(SLabel {
                        id: "label-name".into(),
                        type_: "label:name".into(),
                        text: "P".into(),
                        ..Default::default()
                    })],
                    ..Default::default()
                })],
                ..Default::default()
            })],
            ..Default::default()
        };

        let diags = vec![Diagnostic {
            severity: Severity::Warning,
            code: None,
            message: "x".into(),
            span: Some(sysml_span::Span {
                file: "t.sysml".into(),
                start: 0,
                end: 1,
                line: Some(5),
                col: Some(1),
            }),
            notes: vec![],
            related: vec![],
            tags: vec![],
            tier: sysml_span::DiagnosticTier::default(),
        }];

        assert_eq!(overlay_diagnostics(&mut sgraph, &diags), 1);

        let node = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected node"),
        };
        let comp = match &node.children[0] {
            SModelElement::Compartment(c) => c,
            _ => panic!("expected compartment"),
        };
        let label = match &comp.children[0] {
            SModelElement::Label(l) => l,
            _ => panic!("expected label"),
        };
        assert!(label.diagnostic.is_some());
    }
}
