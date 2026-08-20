#![allow(clippy::indexing_slicing)]
use std::collections::HashMap;

use sysml_core::element_ordering::{primary_span, sort_elements_by_source_order};
use sysml_core::{Element, ElementKind, ModelGraph};
use sysml_span::{Diagnostic, Span};

use super::port::PortRegistry;

/// Resolve a flow endpoint name, supporting dot-path notation of any depth
/// (e.g. "tank.waterOut", "circuit1.breaker.phaseIn").
///
/// Returns true if the endpoint resolves to a known element. For simple names,
/// checks if any element has that name. For dot-paths, walks segment by
/// segment: at each hop the next segment is looked up among the current
/// element's own children and the children of its declared typing definition
/// (`part waterTank : WaterTankWithPorts` — the port lives on the def, not the
/// usage). Cross-file defs need the workspace-merged graph.
fn resolve_endpoint(graph: &ModelGraph, endpoint: &str) -> bool {
    // Simple name — direct match
    if !endpoint.contains('.') {
        return graph
            .elements
            .values()
            .any(|e| e.name.as_deref() == Some(endpoint));
    }

    let segments: Vec<&str> = endpoint.split('.').collect();
    let base_name = segments[0];

    // Find elements named after the base
    let base_elements: Vec<&Element> = graph
        .elements
        .values()
        .filter(|e| e.name.as_deref() == Some(base_name))
        .collect();

    // Segment-wise walk. The frontier holds every element the path so far
    // could denote; each hop matches the next segment among children of the
    // frontier elements and children of their typing definitions.
    let mut frontier: Vec<&Element> = base_elements.clone();
    for segment in &segments[1..] {
        let mut next: Vec<&Element> = Vec::new();
        for elem in &frontier {
            // Own children
            next.extend(
                graph
                    .children_of(&elem.id)
                    .filter(|child| child.name.as_deref() == Some(*segment)),
            );
            // Children of the declared type (last segment of the possibly
            // qualified name)
            if let Some(type_name) = elem
                .get_prop("unresolved_type")
                .and_then(|v| v.as_str())
                .map(|q| q.rsplit("::").next().unwrap_or(q))
            {
                for def in graph
                    .elements
                    .values()
                    .filter(|e| e.name.as_deref() == Some(type_name))
                {
                    next.extend(
                        graph
                            .children_of(&def.id)
                            .filter(|child| child.name.as_deref() == Some(*segment)),
                    );
                }
            }
        }
        if next.is_empty() {
            frontier.clear();
            break;
        }
        frontier = next;
    }
    if !frontier.is_empty() {
        return true;
    }

    // If the base part exists, also check if the final segment exists anywhere
    // in the model (it might be inherited from a type definition).
    // This is intentionally lenient — the health checker flags clear mistakes
    // while deferring complex type resolution to the full resolution pass.
    if !base_elements.is_empty() {
        if let Some(last) = segments.last() {
            let child_exists = graph
                .elements
                .values()
                .any(|e| e.name.as_deref() == Some(*last));
            if child_exists {
                return true;
            }
        }
    }

    // Fallback: check if the full dot-path exists as a literal element name
    graph
        .elements
        .values()
        .any(|e| e.name.as_deref() == Some(endpoint))
}

/// Diagnose flow health issues across all flow elements in a graph.
///
/// This pass is intended for editor diagnostics and preflight checks before
/// interactive simulation.
pub fn flow_health_diagnostics(graph: &ModelGraph) -> Vec<Diagnostic> {
    let flow_kinds = [
        ElementKind::FlowUsage,
        ElementKind::SuccessionFlowUsage,
        ElementKind::Flow,
    ];

    let mut flows: Vec<&Element> = flow_kinds
        .iter()
        .flat_map(|kind| graph.elements_by_kind(kind))
        .collect();
    sort_elements_by_source_order(&mut flows);

    let mut diagnostics = Vec::new();
    // Track source endpoints for multicast detection (FL004).
    // Keep a representative span so LSP can associate the warning with a file/line.
    let mut source_endpoints: HashMap<String, (Vec<String>, Option<Span>)> = HashMap::new();

    for element in &flows {
        let source = element.get_prop("source").and_then(|v| v.as_str());
        let target = element.get_prop("target").and_then(|v| v.as_str());

        // A readable label for an anonymous flow — its endpoints, never the raw
        // ElementId. A UUID in user-facing diagnostics is unactionable.
        let name = element.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| {
            match (source, target) {
                (Some(s), Some(t)) => format!("{s} → {t}"),
                (Some(s), None) => format!("from {s}"),
                (None, Some(t)) => format!("to {t}"),
                (None, None) => "<anonymous>".to_owned(),
            }
        });

        // Check if this flow is typed by a FlowDefinition — typed flows may
        // inherit endpoints from their definition, so missing endpoints are
        // only informational rather than errors.
        // The typing is stored as a FeatureTyping child element with an
        // unresolved_type prop, not directly on the flow element.
        let has_flow_typing = element.get_prop("unresolved_type").is_some()
            || graph.children_of(&element.id).any(|child| {
                child.kind == ElementKind::FeatureTyping
                    && child.get_prop("unresolved_type").is_some()
            });

        // Succession flows order other flows — they don't require explicit
        // port endpoints. Treat missing endpoints as informational.
        let is_succession_flow = element.kind == ElementKind::SuccessionFlowUsage;

        // FL001: Missing source
        if source.is_none() {
            if has_flow_typing || is_succession_flow {
                diagnostics.push(
                    Diagnostic::info(format!("flow '{}' has no explicit source endpoint", name))
                        .with_code("FL001")
                        .with_span(primary_span(element))
                        .with_note(if is_succession_flow {
                            "succession flows order other flows and may not need explicit endpoints"
                        } else {
                            "this flow is typed by a definition that may provide endpoints"
                        })
                        .with_note("to make endpoints explicit, add `from <port>`"),
                );
            } else {
                diagnostics.push(
                    Diagnostic::error(format!("flow '{}' missing source endpoint", name))
                        .with_code("FL001")
                        .with_span(primary_span(element))
                        .with_note("add `from <port>` to specify the source endpoint")
                        .with_note(
                            "example: `flow myFlow from sender.outPort to receiver.inPort;`",
                        ),
                );
            }
        }

        // FL002: Missing target
        if target.is_none() {
            if has_flow_typing || is_succession_flow {
                diagnostics.push(
                    Diagnostic::info(format!("flow '{}' has no explicit target endpoint", name))
                        .with_code("FL002")
                        .with_span(primary_span(element))
                        .with_note(if is_succession_flow {
                            "succession flows order other flows and may not need explicit endpoints"
                        } else {
                            "this flow is typed by a definition that may provide endpoints"
                        })
                        .with_note("to make endpoints explicit, add `to <port>`"),
                );
            } else {
                diagnostics.push(
                    Diagnostic::error(format!("flow '{}' missing target endpoint", name))
                        .with_code("FL002")
                        .with_span(primary_span(element))
                        .with_note("add `to <port>` to specify the target endpoint")
                        .with_note(
                            "example: `flow myFlow from sender.outPort to receiver.inPort;`",
                        ),
                );
            }
        }

        // FL003: Self-loop (source == target)
        if let (Some(src), Some(tgt)) = (source, target) {
            if src == tgt {
                diagnostics.push(
                    Diagnostic::warning(format!("flow '{}' has same source and target", name))
                        .with_code("FL003")
                        .with_span(primary_span(element)),
                );
            }

            // Track source endpoints for multicast detection
            let entry = source_endpoints
                .entry(src.to_owned())
                .or_insert_with(|| (Vec::new(), None));
            entry.0.push(name.clone());
            if entry.1.is_none() {
                let span = primary_span(element);
                if !span.file.is_empty() {
                    entry.1 = Some(span);
                }
            }
        }

        // FL005: Unknown payload type
        if let Some(payload_type) = element.get_prop("payloadType").and_then(|v| v.as_str()) {
            let type_exists = graph
                .elements
                .values()
                .any(|e| e.name.as_deref() == Some(payload_type));
            if !type_exists {
                diagnostics.push(
                    Diagnostic::warning(format!(
                        "flow '{}' references unknown payload type '{}'",
                        name, payload_type
                    ))
                    .with_code("FL005")
                    .with_span(primary_span(element)),
                );
            }
        }

        // FL006: SuccessionFlowUsage informational
        if element.kind == ElementKind::SuccessionFlowUsage {
            diagnostics.push(
                Diagnostic::info(format!(
                    "flow '{}' is a succession flow \u{2014} delivery blocked until source completes",
                    name
                ))
                .with_code("FL006")
                .with_span(primary_span(element))
                .with_note("succession flows enforce ordering between actions")
                .with_note("if ordering is not intended, use `flow` instead of `succession flow`"),
            );
        }

        // FL007: Endpoint not resolvable
        // Supports dot-path endpoints like "waterTank.waterOut" by checking
        // if the base part exists and contains the nested feature.
        if let Some(src) = source {
            if !resolve_endpoint(graph, src) {
                diagnostics.push(
                    Diagnostic::warning(format!(
                        "flow '{}' endpoint '{}' does not resolve to a known part or port",
                        name, src
                    ))
                    .with_code("FL007")
                    .with_span(primary_span(element))
                    .with_note("for dot-path endpoints like `tank.waterOut`, ensure part `tank` exists and contains port/feature `waterOut`")
                    .with_note("check spelling and that the referenced part is visible in this scope"),
                );
            }
        }
        if let Some(tgt) = target {
            if !resolve_endpoint(graph, tgt) {
                diagnostics.push(
                    Diagnostic::warning(format!(
                        "flow '{}' endpoint '{}' does not resolve to a known part or port",
                        name, tgt
                    ))
                    .with_code("FL007")
                    .with_span(primary_span(element))
                    .with_note("for dot-path endpoints like `tank.waterOut`, ensure part `tank` exists and contains port/feature `waterOut`")
                    .with_note("check spelling and that the referenced part is visible in this scope"),
                );
            }
        }

        // FL008: Payload type hint (informational)
        if let Some(payload_type) = element.get_prop("payloadType").and_then(|v| v.as_str()) {
            if let (Some(src), Some(tgt)) = (source, target) {
                let src_has_type = graph
                    .elements
                    .values()
                    .filter(|e| e.name.as_deref() == Some(src))
                    .any(|e| e.get_prop("unresolved_type").is_some());
                let tgt_has_type = graph
                    .elements
                    .values()
                    .filter(|e| e.name.as_deref() == Some(tgt))
                    .any(|e| e.get_prop("unresolved_type").is_some());
                if !src_has_type && !tgt_has_type {
                    diagnostics.push(
                        Diagnostic::info(format!(
                            "flow '{}' has payload type '{}' but endpoints lack matching feature type",
                            name, payload_type
                        ))
                        .with_code("FL008")
                        .with_span(primary_span(element))
                        .with_note("you can add a type annotation to the endpoint ports to match")
                        .with_note(format!("example: `port outPort : {};`", payload_type)),
                    );
                }
            }
        }

        // FL009: Succession flow source not in action (warning)
        if element.kind == ElementKind::SuccessionFlowUsage {
            if let Some(src) = source {
                let src_in_action =
                    graph
                        .elements_by_kind(&ElementKind::ActionDefinition)
                        .any(|action_def| {
                            graph
                                .children_of(&action_def.id)
                                .any(|child| child.name.as_deref() == Some(src))
                        });
                if !src_in_action {
                    diagnostics.push(
                        Diagnostic::warning(format!(
                            "succession flow '{}' source action not found in any action definition",
                            name
                        ))
                        .with_code("FL009")
                        .with_span(primary_span(element)),
                    );
                }
            }
        }
    }

    // FL004: Multiple flows from same source endpoint
    for (source, (flow_names, first_span)) in &source_endpoints {
        if flow_names.len() > 1 {
            let mut sorted_flows = flow_names.clone();
            sorted_flows.sort();
            let listed: Vec<_> = sorted_flows.iter().take(3).cloned().collect();
            let overflow = sorted_flows.len().saturating_sub(listed.len());
            let listed_suffix = if overflow > 0 {
                format!(", +{} more", overflow)
            } else {
                String::new()
            };
            let diag = Diagnostic::warning(format!(
                "multiple flows from '{}' ({}){} \u{2014} may be unintentional multicast",
                source,
                listed.join(", "),
                listed_suffix
            ))
            .with_code("FL004")
            .with_note(format!(
                "flows sharing source '{}': {}",
                source,
                sorted_flows.join(", ")
            ));
            diagnostics.push(if let Some(span) = first_span.clone() {
                diag.with_span(span)
            } else {
                diag
            });
        }
    }

    diagnostics
}

// ---------------------------------------------------------------------------
// Port-aware diagnostics (FL010-FL015)
// ---------------------------------------------------------------------------

/// Validate port connections in flow topology and produce diagnostics.
///
/// Checks registered flows against the port registry for:
/// - FL010: Port type mismatch (different PortDefinition on connected ports)
/// - FL011: Missing required feature (target port expects a feature the source doesn't provide)
/// - FL012: Conjugation incompatibility
/// - FL013: Unconnected output port (has `out` direction but no flow sources it)
/// - FL014: Direction conflict (out→out or in→in without conjugation)
/// - FL015: Port multiplicity detected (informational)
/// Validate the FlowUsage subset of the classified [`LinkGraph`] against the
/// port registry (RSC-3.5e.5 W2 — replaces the former `&[FlowConnectionIR]`
/// inputs; the flow's display id is recovered from the graph via
/// [`LinkIR::display_label`]).
///
/// Ports are classified through the physics domain registry
/// (`classify_port_definition`). FL013 states the Modelica open-terminal
/// assumption for CLASSIFIED POWER ports with a flow feature that appear in no
/// flow: *"open terminal '<part.port>': assuming zero <flow-feature>
/// (unconnected power port)"* — matching the implicit `flow = 0` equation the
/// physics network generates. RSC-1.6: when the flow feature carries a literal
/// numeric `default`, the message becomes *"assuming <value> <flow-feature>
/// (declared default — model boundary condition)"*, matching the pinned
/// FlowSource value. Signal ports and unclassifiable ports keep the legacy
/// "output port not sourced by any flow" informational wording.
pub fn port_health_diagnostics(
    links: &crate::links::LinkGraph,
    registry: &PortRegistry,
    graph: &ModelGraph,
) -> Vec<Diagnostic> {
    port_health_diagnostics_impl(links, registry, graph)
}

fn port_health_diagnostics_impl(
    links: &crate::links::LinkGraph,
    registry: &PortRegistry,
    model: &ModelGraph,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut sourced_ports: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Ports touched by ANY flow endpoint (source or target). Used for the
    // power-port open-terminal check, which is direction-agnostic (Modelica
    // semantics: a connector is "open" when it appears in no connection).
    let mut connected_ports: std::collections::HashSet<String> = std::collections::HashSet::new();

    // RSC-3.5e.5 W2: iterate the FlowUsage subset of the classified link graph
    // (the connector-only links are excluded — they were never in the legacy
    // `flow_connections` input). Interning order == compile_flows order, so the
    // diagnostic order is byte-identical.
    for link in links
        .iter()
        .filter(|l| l.kind == crate::links::LinkSourceKind::FlowUsage)
    {
        let source_key = link.source.key();
        let target_key = link.target.key();
        let flow_label = link.display_label(model);
        sourced_ports.insert(source_key.clone());
        connected_ports.insert(source_key.clone());
        connected_ports.insert(target_key.clone());

        let source_port = registry.get(&source_key);
        let target_port = registry.get(&target_key);

        // Both ports must exist in registry for port-level checks
        let (Some(src), Some(tgt)) = (source_port, target_port) else {
            continue;
        };

        // FL010: Port type mismatch
        if let (Some(src_def), Some(tgt_def)) = (&src.definition, &tgt.definition) {
            if src_def != tgt_def {
                // Check if one is the conjugate of the other (same base name)
                let is_conjugate_pair = src.is_conjugated != tgt.is_conjugated
                    && (src_def == tgt_def
                        || src_def.trim_start_matches('~') == tgt_def.trim_start_matches('~'));
                if !is_conjugate_pair {
                    diagnostics.push(
                        Diagnostic::warning(format!(
                            "FL010: flow '{}' connects port '{}' (type '{}') to '{}' (type '{}') — type mismatch",
                            flow_label, source_key, src_def, target_key, tgt_def
                        ))
                        .with_code("FL010"),
                    );
                }
            }
        }

        // FL011: Missing required features
        for feat_name in tgt.features.keys() {
            if !src.features.contains_key(feat_name) {
                diagnostics.push(
                    Diagnostic::info(format!(
                        "FL011: target port '{}' expects feature '{}' not present on source '{}'",
                        target_key, feat_name, source_key
                    ))
                    .with_code("FL011"),
                );
            }
        }

        // FL014: Direction conflict
        let src_dir = src.effective_direction();
        let tgt_dir = tgt.effective_direction();
        if !src_dir.is_compatible_with(&tgt_dir) {
            diagnostics.push(
                Diagnostic::warning(format!(
                    "FL014: flow '{}' has direction conflict: '{}' is {} and '{}' is {} — \
                     connect out→in, or type the receiving side with a conjugated port \
                     (~PortDef) — out flips to in under conjugation",
                    flow_label, source_key, src_dir, target_key, tgt_dir
                ))
                .with_code("FL014"),
            );
        }
    }

    // FL016: Structural compatibility check across all flows
    diagnostics.extend(registry.validate_connections(links, model));

    // FL013: Unconnected output ports and FL015: Multiplicity info.
    // These informational diagnostics don't carry spans (PortRegistry doesn't
    // store source spans). They're omitted from the LSP pipeline by
    // port_health_diagnostics_from_graph() filtering, but appear in CLI output
    // via `sysml flow` which calls port_health_diagnostics() directly.
    //
    // RSC-1.3: when the model graph is available, classified POWER ports with
    // a flow feature get the open-terminal wording (stating the implicit
    // flow = 0 assumption); signal/unclassified ports keep the legacy rule.
    let phys_registry = crate::physics::domain::PhysicsDomainRegistry::from_workspace_graph(model);
    for (_key, port) in registry.iter() {
        let port_key = port.key();

        // Power-port open-terminal check.
        let mut handled_as_power_port = false;
        {
            let phys = &phys_registry;
            let def_name = port.definition.clone().or_else(|| {
                crate::physics::connection::find_port_definition_for_name(&port.name, model)
            });
            if let Some(def_name) = def_name {
                let classification =
                    crate::physics::classify::classify_port_definition(&def_name, model, phys);
                if classification.confidence
                    != crate::physics::classify::ClassificationConfidence::Unknown
                    && !classification.is_signal
                {
                    if let Some(flow_feat) = classification
                        .features
                        .iter()
                        .find(|f| f.role == crate::physics::domain::VariableRole::Flow)
                    {
                        // This port participates in the physics network. It is
                        // either connected (no diagnostic — the network handles
                        // it) or an open terminal (state the zero-flow
                        // assumption). The legacy "not sourced" wording never
                        // applies to power ports.
                        handled_as_power_port = true;
                        if !connected_ports.contains(&port_key) {
                            // RSC-1.6: a literal numeric default on the FLOW
                            // feature is a declared boundary condition — the
                            // physics network pins the flow to it instead of 0.
                            // Mirror that assumption here.
                            use crate::physics::constraints::{
                                flow_feature_declared_default, DeclaredFlowDefault,
                            };
                            let message = match flow_feature_declared_default(
                                &def_name,
                                &flow_feat.name,
                                model,
                            ) {
                                DeclaredFlowDefault::Numeric(v) => format!(
                                    "FL013: open terminal '{}': assuming {} {} \
                                     (declared default — model boundary condition)",
                                    port_key, v, flow_feat.name,
                                ),
                                DeclaredFlowDefault::NonNumeric => format!(
                                    "FL013: open terminal '{}': assuming zero {} \
                                     (unconnected power port; declared default is \
                                     not a literal numeric — ignored)",
                                    port_key, flow_feat.name,
                                ),
                                DeclaredFlowDefault::None => format!(
                                    "FL013: open terminal '{}': assuming zero {} \
                                     (unconnected power port)",
                                    port_key, flow_feat.name,
                                ),
                            };
                            diagnostics.push(Diagnostic::info(message).with_code("FL013"));
                        }
                    }
                }
            }
        }

        if !handled_as_power_port {
            let dir = port.effective_direction();
            if (dir == super::port::PortDirection::Out || dir == super::port::PortDirection::InOut)
                && !sourced_ports.contains(&port_key)
            {
                diagnostics.push(
                    Diagnostic::info(format!(
                        "FL013: output port '{}' is not sourced by any flow",
                        port_key,
                    ))
                    .with_code("FL013"),
                );
            }
        }

        if let Some(n) = port.multiplicity {
            diagnostics.push(
                Diagnostic::info(format!(
                    "FL015: port '{}' has multiplicity [{}]",
                    port.key(),
                    n,
                ))
                .with_code("FL015"),
            );
        }
    }

    diagnostics
}

/// Convenience wrapper: compile ports and flows from a ModelGraph, then run
/// port health diagnostics.
///
/// This matches the `fn(&ModelGraph) -> Vec<Diagnostic>` signature expected
/// by the LSP health diagnostic pipeline. The LSP needs every diagnostic
/// to carry a span (see `diagnostic_ux_tests::fixture_corpus_has_no_spanless_
/// diagnostics`), but `PortRegistry` doesn't track element spans, so the
/// CLI-only informational checks (FL013 unconnected output port, FL015
/// port multiplicity) come out spanless. Filter them here so they stay on
/// the CLI side (`sysml flow` calls `port_health_diagnostics` directly).
pub fn port_health_diagnostics_from_graph(graph: &ModelGraph) -> Vec<Diagnostic> {
    let registry = super::compile_ports(graph);
    // RSC-3.5e.5 W3: classify_links now folds in the flow producer, so it walks
    // the flow elements itself — the FlowUsage subset is the former flow list.
    let (links, _diags) = crate::links::classify_links(graph, &registry);
    port_health_diagnostics(&links, &registry, graph)
        .into_iter()
        .filter(|d| d.span.is_some())
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::super::port::{PortDirection, PortFeature, PortInstanceIR};
    use super::*;
    use sysml_core::Element;

    #[test]
    fn reports_flow_missing_source() {
        let mut graph = ModelGraph::new();
        let flow = Element::new_with_kind(ElementKind::FlowUsage)
            .with_name("dataFlow")
            .with_prop("target", "controller.input")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(flow);

        let diagnostics = flow_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("FL001")
                && d.message.contains("dataFlow")
                && d.message.contains("missing source")));
    }

    #[test]
    fn reports_flow_missing_target() {
        let mut graph = ModelGraph::new();
        let flow = Element::new_with_kind(ElementKind::FlowUsage)
            .with_name("dataFlow")
            .with_prop("source", "sensor.output")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(flow);

        let diagnostics = flow_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("FL002")
                && d.message.contains("dataFlow")
                && d.message.contains("missing target")));
    }

    #[test]
    fn reports_flow_self_loop() {
        let mut graph = ModelGraph::new();
        let flow = Element::new_with_kind(ElementKind::FlowUsage)
            .with_name("loopFlow")
            .with_prop("source", "a.out")
            .with_prop("target", "a.out")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(flow);

        let diagnostics = flow_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("FL003")
                && d.message.contains("loopFlow")
                && d.message.contains("same source and target")));
    }

    #[test]
    fn reports_multicast_warning() {
        let mut graph = ModelGraph::new();

        let f1 = Element::new_with_kind(ElementKind::FlowUsage)
            .with_name("flow1")
            .with_prop("source", "sensor.data")
            .with_prop("target", "controller.input")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(f1);

        let f2 = Element::new_with_kind(ElementKind::FlowUsage)
            .with_name("flow2")
            .with_prop("source", "sensor.data")
            .with_prop("target", "logger.input")
            .with_span(Span::new("file:///test.sysml", 11, 20));
        graph.add_element(f2);

        let diagnostics = flow_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("FL004")
                && d.message.contains("sensor.data")
                && d.message.contains("flow1")
                && d.message.contains("flow2")
                && d.message.contains("multicast")));
        let fl004 = diagnostics
            .iter()
            .find(|d| d.code.as_deref() == Some("FL004"))
            .expect("FL004 should be emitted");
        assert!(
            fl004.span.is_some(),
            "FL004 should carry a span so editors can place it at flow location"
        );
        let note = fl004
            .notes
            .iter()
            .find(|n| n.contains("flows sharing source"))
            .expect("FL004 should include flow list note");
        assert!(
            note.contains("flow1") && note.contains("flow2"),
            "FL004 note should include implicated flow names: {note}"
        );
    }

    #[test]
    fn reports_unknown_payload_type() {
        let mut graph = ModelGraph::new();

        let flow = Element::new_with_kind(ElementKind::FlowUsage)
            .with_name("typedFlow")
            .with_prop("source", "a.out")
            .with_prop("target", "b.in")
            .with_prop("payloadType", "NonExistentType")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(flow);

        let diagnostics = flow_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("FL005")
                && d.message.contains("typedFlow")
                && d.message.contains("NonExistentType")));
    }

    #[test]
    fn reports_succession_flow_info() {
        let mut graph = ModelGraph::new();

        let flow = Element::new_with_kind(ElementKind::SuccessionFlowUsage)
            .with_name("orderedFlow")
            .with_prop("source", "step1.out")
            .with_prop("target", "step2.in")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(flow);

        let diagnostics = flow_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("FL006")
                && d.message.contains("orderedFlow")
                && d.message.contains("succession flow")));
    }

    #[test]
    fn no_diagnostics_for_valid_flow() {
        let mut graph = ModelGraph::new();

        // Add a type definition so payload type resolves (FL005)
        let type_def = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("Temperature")
            .with_span(Span::new("file:///test.sysml", 0, 5));
        graph.add_element(type_def);

        // Add source and target elements so endpoints resolve (FL007)
        // and give them unresolved_type so FL008 doesn't fire
        let src_elem = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("sensor.reading")
            .with_prop("unresolved_type", "Temperature")
            .with_span(Span::new("file:///test.sysml", 6, 10));
        graph.add_element(src_elem);

        let tgt_elem = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("controller.input")
            .with_prop("unresolved_type", "Temperature")
            .with_span(Span::new("file:///test.sysml", 11, 15));
        graph.add_element(tgt_elem);

        let flow = Element::new_with_kind(ElementKind::FlowUsage)
            .with_name("tempFlow")
            .with_prop("source", "sensor.reading")
            .with_prop("target", "controller.input")
            .with_prop("payloadType", "Temperature")
            .with_span(Span::new("file:///test.sysml", 16, 30));
        graph.add_element(flow);

        let diagnostics = flow_health_diagnostics(&graph);
        // Should have no errors or warnings (only info-level at most for non-succession)
        assert!(
            !diagnostics.iter().any(|d| d
                .code
                .as_deref()
                .map_or(false, |c| c.starts_with("FL00") && c != "FL006")),
            "valid flow should produce no error/warning diagnostics, got: {:?}",
            diagnostics
        );
    }

    #[test]
    fn reports_unresolvable_endpoint() {
        let mut graph = ModelGraph::new();
        let flow = Element::new_with_kind(ElementKind::FlowUsage)
            .with_name("badFlow")
            .with_prop("source", "nonExistentPart")
            .with_prop("target", "alsoNonExistent")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(flow);

        let diagnostics = flow_health_diagnostics(&graph);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("FL007")),
            "expected FL007 for unresolvable endpoint, got: {:?}",
            diagnostics
        );
    }

    /// Dot-path endpoints resolve through the part's typing definition:
    /// `part waterTank : WaterTankWithPorts` + `port waterOut` declared on
    /// the def (not the usage) must satisfy `waterTank.waterOut`.
    /// Regression for the coffee-machine fixture FL007 false positive.
    #[test]
    fn endpoint_resolves_through_part_typing_definition() {
        let mut graph = ModelGraph::new();

        // The typing definition with its port child.
        let def = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("WaterTankWithPorts")
            .with_span(Span::new("file:///ports.sysml", 0, 10));
        let def_id = graph.add_element(def);

        let port = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("waterOut")
            .with_owner(def_id)
            .with_span(Span::new("file:///ports.sysml", 11, 20));
        graph.add_element(port);

        // The part usage typed by the definition (qualified name on the
        // prop, as the parser records it).
        let part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("waterTank")
            .with_prop("unresolved_type", "Ports::WaterTankWithPorts")
            .with_span(Span::new("file:///flows.sysml", 0, 10));
        graph.add_element(part);

        // Target endpoint exists directly so only the source exercises
        // the typing traversal.
        let sink = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("brewer")
            .with_span(Span::new("file:///flows.sysml", 11, 15));
        let sink_id = graph.add_element(sink);
        let sink_port = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("waterIn")
            .with_owner(sink_id)
            .with_span(Span::new("file:///flows.sysml", 16, 20));
        graph.add_element(sink_port);

        let flow = Element::new_with_kind(ElementKind::FlowUsage)
            .with_name("waterToBrewer")
            .with_prop("source", "waterTank.waterOut")
            .with_prop("target", "brewer.waterIn")
            .with_span(Span::new("file:///flows.sysml", 21, 40));
        graph.add_element(flow);

        let diagnostics = flow_health_diagnostics(&graph);
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("FL007")),
            "endpoint typed via part's definition must not FL007, got: {:?}",
            diagnostics
        );
    }

    /// Three-segment dot-paths walk part → nested part (via typing def) →
    /// port (via the nested part's typing def):
    /// `circuit1.breaker.phaseIn` where `circuit1 : CircuitPath`,
    /// CircuitPath owns `breaker : DualPoleBreaker`, and DualPoleBreaker
    /// owns `phaseIn`. Regression for the multi-circuit fixture FL007 ×10.
    #[test]
    fn endpoint_resolves_multi_segment_through_nested_typing() {
        let mut graph = ModelGraph::new();

        // port def target: DualPoleBreaker with port phaseIn
        let breaker_def = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("DualPoleBreaker")
            .with_span(Span::new("file:///circuit.sysml", 0, 10));
        let breaker_def_id = graph.add_element(breaker_def);
        let phase_in = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("phaseIn")
            .with_owner(breaker_def_id)
            .with_span(Span::new("file:///circuit.sysml", 11, 18));
        graph.add_element(phase_in);

        // CircuitPath def owns `breaker : DualPoleBreaker`
        let circuit_def = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("CircuitPath")
            .with_span(Span::new("file:///circuit.sysml", 19, 30));
        let circuit_def_id = graph.add_element(circuit_def);
        let breaker_usage = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("breaker")
            .with_prop("unresolved_type", "DualPoleBreaker")
            .with_owner(circuit_def_id)
            .with_span(Span::new("file:///circuit.sysml", 31, 40));
        graph.add_element(breaker_usage);

        // top-level usage `circuit1 : CircuitPath`
        let circuit1 = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("circuit1")
            .with_prop("unresolved_type", "CircuitPathStructure::CircuitPath")
            .with_span(Span::new("file:///board.sysml", 0, 10));
        graph.add_element(circuit1);

        // source endpoint exists directly
        let busbar = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("busbar")
            .with_span(Span::new("file:///board.sysml", 11, 17));
        let busbar_id = graph.add_element(busbar);
        let tap = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("circuitOut1")
            .with_owner(busbar_id)
            .with_span(Span::new("file:///board.sysml", 18, 28));
        graph.add_element(tap);

        let flow = Element::new_with_kind(ElementKind::FlowUsage)
            .with_name("busbarToCircuit1")
            .with_prop("source", "busbar.circuitOut1")
            .with_prop("target", "circuit1.breaker.phaseIn")
            .with_span(Span::new("file:///board.sysml", 29, 60));
        graph.add_element(flow);

        let diagnostics = flow_health_diagnostics(&graph);
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("FL007")),
            "3-segment endpoint via nested typing must not FL007, got: {:?}",
            diagnostics
        );

        // Strict-walk check on the resolver itself: a bogus middle segment
        // still fails the walk (the diagnostic-level pass above could also
        // be satisfied by the lenient tail).
        assert!(resolve_endpoint(&graph, "circuit1.breaker.phaseIn"));
        assert!(!resolve_endpoint(&graph, "circuitX.breaker.phaseIn"));
    }

    #[test]
    fn reports_payload_type_mismatch() {
        let mut graph = ModelGraph::new();

        // Add source and target elements without unresolved_type
        let src_part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("srcPort")
            .with_span(Span::new("file:///test.sysml", 0, 5));
        graph.add_element(src_part);

        let tgt_part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("tgtPort")
            .with_span(Span::new("file:///test.sysml", 6, 10));
        graph.add_element(tgt_part);

        // Add type definition so FL005 doesn't fire
        let type_def = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("Signal")
            .with_span(Span::new("file:///test.sysml", 11, 15));
        graph.add_element(type_def);

        let flow = Element::new_with_kind(ElementKind::FlowUsage)
            .with_name("typedFlow")
            .with_prop("source", "srcPort")
            .with_prop("target", "tgtPort")
            .with_prop("payloadType", "Signal")
            .with_span(Span::new("file:///test.sysml", 16, 30));
        graph.add_element(flow);

        let diagnostics = flow_health_diagnostics(&graph);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("FL008")),
            "expected FL008 for payload type mismatch, got: {:?}",
            diagnostics
        );
    }

    #[test]
    fn reports_succession_flow_source_not_in_action() {
        let mut graph = ModelGraph::new();

        // Add source element so FL007 doesn't fire
        let src_part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("step1")
            .with_span(Span::new("file:///test.sysml", 0, 5));
        graph.add_element(src_part);

        let tgt_part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("step2")
            .with_span(Span::new("file:///test.sysml", 6, 10));
        graph.add_element(tgt_part);

        let flow = Element::new_with_kind(ElementKind::SuccessionFlowUsage)
            .with_name("seqFlow")
            .with_prop("source", "step1")
            .with_prop("target", "step2")
            .with_span(Span::new("file:///test.sysml", 11, 20));
        graph.add_element(flow);

        let diagnostics = flow_health_diagnostics(&graph);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("FL009")),
            "expected FL009 for succession flow source not in action, got: {:?}",
            diagnostics
        );
    }

    #[test]
    fn succession_flow_without_endpoints_triggers_fl001_fl002() {
        let mut graph = ModelGraph::new();
        let flow = Element::new_with_kind(ElementKind::SuccessionFlowUsage)
            .with_name("emptySuccFlow")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(flow);

        let diagnostics = flow_health_diagnostics(&graph);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("FL001")),
            "SuccessionFlowUsage without source should trigger FL001, got: {:?}",
            diagnostics
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("FL002")),
            "SuccessionFlowUsage without target should trigger FL002, got: {:?}",
            diagnostics
        );
    }

    // -----------------------------------------------------------------------
    // Port diagnostics tests (FL010-FL015)
    // -----------------------------------------------------------------------

    fn water_port(owner: &str, name: &str, dir: PortDirection) -> PortInstanceIR {
        let mut p = PortInstanceIR::new(owner, name)
            .with_definition("WaterPort")
            .with_direction(dir);
        p.add_feature(PortFeature {
            name: "flowRate".into(),
            direction: dir,
            type_name: Some("Real".into()),
            value: sysml_core::Value::Float(0.0),
        });
        p
    }

    /// RSC-3.5e.5 W2: build a one-link FlowUsage `LinkGraph` (the input shape
    /// the port diagnostics now consume). The element id is `flow:{id}` so the
    /// graph-less tests fall back to that string for `display_label` — the FL
    /// *code* is what these tests assert, not the message text.
    fn one_flow_lg(
        id: &str,
        src_part: &str,
        src_port: &str,
        tgt_part: &str,
        tgt_port: &str,
    ) -> crate::links::LinkGraph {
        use crate::links::{LinkClass, LinkEndpoint, LinkGraph, LinkIR, LinkSourceKind};
        use sysml_core::physics::classify::ClassificationConfidence;
        let mut lg = LinkGraph::new();
        lg.intern(LinkIR {
            element_id: sysml_core::ElementId::from_string(format!("flow:{id}")),
            kind: LinkSourceKind::FlowUsage,
            source: LinkEndpoint {
                element_id: None,
                owner: src_part.into(),
                port: src_port.into(),
                resolved_registry_key: None,
            },
            target: LinkEndpoint {
                element_id: None,
                owner: tgt_part.into(),
                port: tgt_port.into(),
                resolved_registry_key: None,
            },
            class: LinkClass::MessageChannel,
            class_confidence: ClassificationConfidence::Declared,
            is_succession: false,
            is_move: false,
            is_push: false,
            payload_type: None,
            source_payload_type: None,
            target_payload_type: None,
            via_interface: None,
        });
        lg
    }

    #[test]
    fn fl010_type_mismatch() {
        let mut reg = PortRegistry::new();
        reg.register(water_port("tank", "waterOut", PortDirection::Out));
        reg.register(
            PortInstanceIR::new("heater", "powerIn")
                .with_definition("PowerPort")
                .with_direction(PortDirection::In),
        );

        let links = one_flow_lg("f1", "tank", "waterOut", "heater", "powerIn");
        let diags = port_health_diagnostics(&links, &reg, &ModelGraph::new());
        assert!(diags.iter().any(|d| d.code.as_deref() == Some("FL010")));
    }

    #[test]
    fn fl011_missing_feature() {
        let mut reg = PortRegistry::new();
        // Source has no features
        reg.register(PortInstanceIR::new("sensor", "out").with_direction(PortDirection::Out));
        // Target expects flowRate
        reg.register(water_port("pump", "in", PortDirection::In));

        let links = one_flow_lg("f1", "sensor", "out", "pump", "in");
        let diags = port_health_diagnostics(&links, &reg, &ModelGraph::new());
        assert!(diags.iter().any(|d| d.code.as_deref() == Some("FL011")));
    }

    #[test]
    fn fl014_direction_conflict() {
        let mut reg = PortRegistry::new();
        reg.register(water_port("tank", "waterOut", PortDirection::Out));
        reg.register(water_port("brewer", "waterOut2", PortDirection::Out));

        let links = one_flow_lg("f1", "tank", "waterOut", "brewer", "waterOut2");
        let diags = port_health_diagnostics(&links, &reg, &ModelGraph::new());
        assert!(diags.iter().any(|d| d.code.as_deref() == Some("FL014")));
    }

    #[test]
    fn no_diagnostics_for_compatible_ports() {
        let mut reg = PortRegistry::new();
        reg.register(water_port("tank", "waterOut", PortDirection::Out));
        reg.register(water_port("brewer", "waterIn", PortDirection::In));

        let links = one_flow_lg("f1", "tank", "waterOut", "brewer", "waterIn");
        let diags = port_health_diagnostics(&links, &reg, &ModelGraph::new());

        // Should have no errors or warnings (FL013 won't fire because tank.waterOut is sourced)
        let errors_warnings: Vec<_> = diags
            .iter()
            .filter(|d| d.code.as_deref() != Some("FL013"))
            .collect();
        assert!(
            errors_warnings.is_empty(),
            "unexpected diagnostics: {:?}",
            errors_warnings
        );
    }

    #[test]
    fn fl015_multiplicity() {
        let mut reg = PortRegistry::new();
        reg.register(
            PortInstanceIR::new("molex", "circuitOut")
                .with_definition("CircuitOutputPort")
                .with_multiplicity(4)
                .with_direction(PortDirection::Out),
        );

        let diags =
            port_health_diagnostics(&crate::links::LinkGraph::new(), &reg, &ModelGraph::new());
        assert!(diags.iter().any(|d| d.code.as_deref() == Some("FL015")));
        assert!(diags.iter().any(|d| d.code.as_deref() == Some("FL013"))); // unconnected
    }

    // ── RSC-1.3: FL013 open-terminal wording for classified power ports ──

    use sysml_core::{ElementId, Value};

    /// Model with a power port def (ISQ voltage+current ⇒ power port) and a
    /// signal port def (flow-only ⇒ signal).
    fn classified_port_model() -> ModelGraph {
        let mut model = ModelGraph::new();

        let power_def = ElementId::new_v4();
        model.add_element(
            Element::new(power_def.clone(), ElementKind::PortDefinition).with_name("ElPowerPort"),
        );
        model.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(power_def.clone())
                .with_name("voltage")
                .with_prop("typeName", Value::String("ElectricPotentialValue".into())),
        );
        model.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(power_def)
                .with_name("current")
                .with_prop("typeName", Value::String("ElectricCurrentValue".into())),
        );

        let sense_def = ElementId::new_v4();
        model.add_element(
            Element::new(sense_def.clone(), ElementKind::PortDefinition).with_name("SensePort"),
        );
        model.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(sense_def)
                .with_name("rms")
                .with_prop("typeName", Value::String("ElectricCurrentValue".into())),
        );

        model
    }

    /// Unconnected classified POWER port: FL013 becomes the open-terminal
    /// info that states the zero-flow assumption.
    #[test]
    fn fl013_power_port_states_zero_flow_assumption() {
        let model = classified_port_model();
        let mut reg = PortRegistry::new();
        reg.register(
            PortInstanceIR::new("busbar", "circuitOut2")
                .with_definition("ElPowerPort")
                .with_direction(PortDirection::Out),
        );

        let diags = port_health_diagnostics(&crate::links::LinkGraph::new(), &reg, &model);
        let fl013 = diags
            .iter()
            .find(|d| d.code.as_deref() == Some("FL013"))
            .expect("FL013 should fire for the open terminal");
        assert_eq!(fl013.severity, sysml_span::Severity::Info);
        assert!(
            fl013.message.contains("open terminal 'busbar.circuitOut2'")
                && fl013.message.contains("assuming zero current")
                && fl013.message.contains("unconnected power port"),
            "power-port FL013 must state the assumption, got: {}",
            fl013.message
        );
    }

    /// Unconnected SIGNAL port keeps the legacy informational wording —
    /// no physics claim, no zero-flow assumption.
    #[test]
    fn fl013_signal_port_keeps_legacy_wording() {
        let model = classified_port_model();
        let mut reg = PortRegistry::new();
        reg.register(
            PortInstanceIR::new("sensor", "senseOut")
                .with_definition("SensePort")
                .with_direction(PortDirection::Out),
        );

        let diags = port_health_diagnostics(&crate::links::LinkGraph::new(), &reg, &model);
        let fl013 = diags
            .iter()
            .find(|d| d.code.as_deref() == Some("FL013"))
            .expect("legacy FL013 should still fire for unconnected signal output");
        assert!(
            fl013.message.contains("is not sourced by any flow"),
            "signal-port FL013 keeps legacy wording, got: {}",
            fl013.message
        );
        assert!(
            !fl013.message.contains("assuming zero"),
            "signal ports must not get the zero-flow assumption"
        );
    }

    /// CONNECTED power port (appears as a flow endpoint — even as a target):
    /// no FL013 of any flavour.
    #[test]
    fn fl013_silent_for_connected_power_port() {
        let model = classified_port_model();
        let mut reg = PortRegistry::new();
        reg.register(
            PortInstanceIR::new("busbar", "circuitOut1")
                .with_definition("ElPowerPort")
                .with_direction(PortDirection::Out),
        );
        reg.register(
            PortInstanceIR::new("load1", "powerIn")
                .with_definition("ElPowerPort")
                .with_direction(PortDirection::In),
        );

        let links = one_flow_lg("f1", "busbar", "circuitOut1", "load1", "powerIn");
        let diags = port_health_diagnostics(&links, &reg, &model);
        assert!(
            !diags.iter().any(|d| d.code.as_deref() == Some("FL013")),
            "connected power ports get no FL013: {:?}",
            diags
        );
    }

    // ── RSC-1.6: FL013 names declared defaults (boundary conditions) ──

    /// [`classified_port_model`] with extra props set on the named feature
    /// (RSC-1.6 declared-default scenarios).
    fn classified_port_model_with_feature_props(
        feature: &str,
        props: &[(&str, Value)],
    ) -> ModelGraph {
        let mut model = classified_port_model();
        let feat_id = model
            .elements
            .values()
            .find(|e| e.name.as_deref() == Some(feature))
            .map(|e| e.id.clone())
            .expect("feature exists in classified_port_model");
        let elem = model.get_element_mut(&feat_id).expect("feature element");
        for (key, value) in props {
            elem.set_prop(key.to_string(), value.clone());
        }
        model
    }

    /// Open POWER terminal whose flow feature declares `default 2.0`:
    /// FL013 names the assumed value and calls it a boundary condition.
    #[test]
    fn fl013_power_port_default_states_boundary_condition() {
        let model = classified_port_model_with_feature_props(
            "current",
            &[
                ("value", Value::Float(2.0)),
                ("isDefault", Value::Bool(true)),
            ],
        );
        let mut reg = PortRegistry::new();
        reg.register(
            PortInstanceIR::new("busbar", "circuitOut2")
                .with_definition("ElPowerPort")
                .with_direction(PortDirection::Out),
        );

        let diags = port_health_diagnostics(&crate::links::LinkGraph::new(), &reg, &model);
        let fl013 = diags
            .iter()
            .find(|d| d.code.as_deref() == Some("FL013"))
            .expect("FL013 should fire for the open terminal");
        assert_eq!(fl013.severity, sysml_span::Severity::Info);
        assert!(
            fl013.message.contains("open terminal 'busbar.circuitOut2'")
                && fl013.message.contains("assuming 2 current")
                && fl013
                    .message
                    .contains("declared default — model boundary condition"),
            "FL013 must name the declared default, got: {}",
            fl013.message
        );
    }

    /// Negative default carried by the legacy `unresolved_value` string is
    /// named correctly in FL013.
    #[test]
    fn fl013_negative_default_via_unresolved_value_string() {
        let model = classified_port_model_with_feature_props(
            "current",
            &[
                ("isDefault", Value::Bool(true)),
                ("unresolved_value", Value::String("-2.5".into())),
            ],
        );
        let mut reg = PortRegistry::new();
        reg.register(
            PortInstanceIR::new("busbar", "circuitOut2")
                .with_definition("ElPowerPort")
                .with_direction(PortDirection::Out),
        );

        let diags = port_health_diagnostics(&crate::links::LinkGraph::new(), &reg, &model);
        let fl013 = diags
            .iter()
            .find(|d| d.code.as_deref() == Some("FL013"))
            .expect("FL013 should fire for the open terminal");
        assert!(
            fl013.message.contains("assuming -2.5 current")
                && fl013.message.contains("declared default"),
            "FL013 must name the negative default, got: {}",
            fl013.message
        );
    }

    /// Non-numeric default → zero-flow wording plus a note that the
    /// declared default was ignored.
    #[test]
    fn fl013_non_numeric_default_noted_and_ignored() {
        let model = classified_port_model_with_feature_props(
            "current",
            &[
                ("isDefault", Value::Bool(true)),
                ("value", Value::String("nominalDraw".into())),
            ],
        );
        let mut reg = PortRegistry::new();
        reg.register(
            PortInstanceIR::new("busbar", "circuitOut2")
                .with_definition("ElPowerPort")
                .with_direction(PortDirection::Out),
        );

        let diags = port_health_diagnostics(&crate::links::LinkGraph::new(), &reg, &model);
        let fl013 = diags
            .iter()
            .find(|d| d.code.as_deref() == Some("FL013"))
            .expect("FL013 should fire for the open terminal");
        assert!(
            fl013.message.contains("assuming zero current")
                && fl013.message.contains("not a literal numeric"),
            "FL013 must note the ignored default, got: {}",
            fl013.message
        );
    }

    /// SIGNAL port with a declared default keeps the legacy wording — no
    /// physics claim, no boundary condition.
    #[test]
    fn fl013_signal_port_with_default_keeps_legacy_wording() {
        let model = classified_port_model_with_feature_props(
            "rms",
            &[
                ("value", Value::Float(3.0)),
                ("isDefault", Value::Bool(true)),
            ],
        );
        let mut reg = PortRegistry::new();
        reg.register(
            PortInstanceIR::new("sensor", "senseOut")
                .with_definition("SensePort")
                .with_direction(PortDirection::Out),
        );

        let diags = port_health_diagnostics(&crate::links::LinkGraph::new(), &reg, &model);
        let fl013 = diags
            .iter()
            .find(|d| d.code.as_deref() == Some("FL013"))
            .expect("legacy FL013 should still fire for unconnected signal output");
        assert!(
            fl013.message.contains("is not sourced by any flow")
                && !fl013.message.contains("declared default"),
            "signal-port FL013 keeps legacy wording, got: {}",
            fl013.message
        );
    }

    /// When the port definition does not classify (here: an empty graph, so
    /// `ElPowerPort` resolves to no physics domain) the FL013 wording stays the
    /// legacy "not sourced by any flow" form — the power-port open-terminal
    /// wording only applies to classified power ports.
    #[test]
    fn fl013_unclassified_port_uses_legacy_wording() {
        let mut reg = PortRegistry::new();
        reg.register(
            PortInstanceIR::new("busbar", "circuitOut2")
                .with_definition("ElPowerPort")
                .with_direction(PortDirection::Out),
        );

        let diags =
            port_health_diagnostics(&crate::links::LinkGraph::new(), &reg, &ModelGraph::new());
        let fl013 = diags
            .iter()
            .find(|d| d.code.as_deref() == Some("FL013"))
            .expect("FL013 fires via legacy rule");
        assert!(fl013.message.contains("is not sourced by any flow"));
    }
}
