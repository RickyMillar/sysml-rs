//! Flow elaboration.
//!
//! Extracts source/target from `ItemFlow`, `FlowUsage`, and `SuccessionFlowUsage`
//! elements. The parser may store endpoints as child elements rather than as
//! direct `source`/`target` string properties. This pass derives the string
//! properties that the flow compiler expects.

use super::ElaborationReport;
use crate::{
    CanonicalKey, ElementId, ElementKind, ModelGraph, Relationship, RelationshipKind, Value,
};

/// Elaborate flows by deriving source/target properties from endpoint children,
/// extracting payload type information, and synthesizing Flow relationships.
pub(super) fn elaborate_flows(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    extract_payload_types(graph, report);
    // Collect flow element IDs via kind index
    let mut flow_candidate_ids = Vec::new();
    flow_candidate_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::FlowUsage));
    flow_candidate_ids
        .extend_from_slice(graph.element_ids_by_kind(&ElementKind::SuccessionFlowUsage));
    flow_candidate_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::Flow));
    flow_candidate_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::FlowDefinition));

    // Collect flow elements that need elaboration
    let flow_infos: Vec<FlowInfo> = flow_candidate_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter(|e| {
            // Only elaborate if source/target not already set as strings
            let has_source = e.get_prop("source").and_then(|v| v.as_str()).is_some();
            let has_target = e.get_prop("target").and_then(|v| v.as_str()).is_some();
            !has_source || !has_target
        })
        .map(|e| FlowInfo {
            id: e.id.clone(),
            has_source: e.get_prop("source").and_then(|v| v.as_str()).is_some(),
            has_target: e.get_prop("target").and_then(|v| v.as_str()).is_some(),
        })
        .collect();

    for info in flow_infos {
        let (source, target) = extract_endpoints_from_children(graph, &info.id);

        let mut changed = false;

        if !info.has_source {
            if let Some(src) = source {
                if let Some(elem) = graph.get_element_mut(&info.id) {
                    elem.set_prop("source", Value::String(src));
                    changed = true;
                }
            }
        }

        if !info.has_target {
            if let Some(tgt) = target {
                if let Some(elem) = graph.get_element_mut(&info.id) {
                    elem.set_prop("target", Value::String(tgt));
                    changed = true;
                }
            }
        }

        if changed {
            report.flows_derived += 1;
        }
    }

    // Phase 2: Synthesize Flow relationships from flow elements with source/target props
    synthesize_flow_relationships(graph, report);
}

/// Create `Relationship::Flow` (or `SuccessionFlow`) from flow elements that have
/// resolved source/target string properties. Resolves names to ElementIds and
/// adds relationships to the graph.
fn synthesize_flow_relationships(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    let mut flow_ids = Vec::new();
    flow_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::FlowUsage));
    flow_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::SuccessionFlowUsage));
    flow_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::Flow));
    flow_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::FlowDefinition));

    super::connectors::stamp_endpoint_ids(graph, report, &flow_ids);

    let flow_rels: Vec<(ElementId, ElementId, ElementId, bool)> = flow_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter_map(|e| {
            let source_name = e.get_prop("source").and_then(|v| v.as_str())?;
            let target_name = e.get_prop("target").and_then(|v| v.as_str())?;
            // Resolve names — skip if they resolve to the flow's own children
            // (endpoint Features are children, not meaningful relationship targets)
            let source_id = super::resolve_name(graph, &e.owner, source_name).filter(|id| {
                graph
                    .get_element(id)
                    .is_none_or(|el| el.owner.as_ref() != Some(&e.id))
            })?;
            let target_id = super::resolve_name(graph, &e.owner, target_name).filter(|id| {
                graph
                    .get_element(id)
                    .is_none_or(|el| el.owner.as_ref() != Some(&e.id))
            })?;
            let is_succession = e.kind == ElementKind::SuccessionFlowUsage;
            Some((e.id.clone(), source_id, target_id, is_succession))
        })
        .collect();

    for (origin_id, source_id, target_id, is_succession) in flow_rels {
        let kind = if is_succession {
            RelationshipKind::SuccessionFlow
        } else {
            RelationshipKind::Flow
        };

        // Check if relationship already exists
        let already_exists = graph
            .relationships_by_kind(&kind)
            .any(|r| r.source == source_id && r.target == target_id);

        if !already_exists {
            let src_key = CanonicalKey::root(&source_id.to_string());
            let tgt_key = CanonicalKey::root(&target_id.to_string());
            let edge_key = CanonicalKey::for_relationship(&src_key, kind.as_str(), &tgt_key, 0);
            let mut rel = Relationship::new_with_key(kind, source_id, target_id, &edge_key);
            rel.props
                .insert("origin_flow".into(), Value::Ref(origin_id));
            graph.add_relationship(rel);
            report.flows_derived += 1;
        }
    }
}

struct FlowInfo {
    id: ElementId,
    has_source: bool,
    has_target: bool,
}

/// Extract payload type from flow elements.
///
/// SysML v2 syntax: `flow of ItemType from a.out to b.in`
/// The `of ItemType` part creates a FeatureTyping child with `unresolved_type`.
/// This function copies that type name to the flow's `payloadType` property.
fn extract_payload_types(graph: &mut ModelGraph, _report: &mut ElaborationReport) {
    // Collect flow elements that need payload type extraction
    let mut payload_flow_ids = Vec::new();
    payload_flow_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::FlowUsage));
    payload_flow_ids
        .extend_from_slice(graph.element_ids_by_kind(&ElementKind::SuccessionFlowUsage));
    payload_flow_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::Flow));
    payload_flow_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::FlowDefinition));

    let to_update: Vec<(ElementId, String)> = payload_flow_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter(|e| e.get_prop("payloadType").is_none())
        .filter_map(|e| {
            // Look for FeatureTyping children that carry the payload type
            let type_name = find_payload_type(graph, &e.id)?;
            Some((e.id.clone(), type_name))
        })
        .collect();

    for (id, type_name) in to_update {
        if let Some(elem) = graph.get_element_mut(&id) {
            elem.set_prop("payloadType", Value::String(type_name));
        }
    }
}

/// Find payload type from a flow element's children.
///
/// Looks for FeatureTyping children (direct or nested) that contain the
/// type of the payload being flowed.
fn find_payload_type(graph: &ModelGraph, flow_id: &ElementId) -> Option<String> {
    for child in graph.children_of(flow_id) {
        // Direct FeatureTyping child with unresolved_type
        if child.kind == ElementKind::FeatureTyping
            || child.kind.is_subtype_of(ElementKind::FeatureTyping)
        {
            if let Some(type_name) = child.get_prop("unresolved_type").and_then(|v| v.as_str()) {
                return Some(type_name.to_owned());
            }
        }

        // Also check non-endpoint Feature children that may carry typing
        // (the payload feature member, not the endpoint features).
        // PayloadFeature is the parser's spec-shaped payload child
        // (SysML.xtext PayloadFeature:1293 — its FeatureTyping carries the
        // `of X` type); Feature/ReferenceUsage cover older store shapes.
        if child.kind == ElementKind::Feature
            || child.kind == ElementKind::ReferenceUsage
            || child.kind == ElementKind::PayloadFeature
        {
            let is_end = child
                .get_prop("isEnd")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if is_end {
                continue; // Skip endpoint features
            }

            // Check this child's FeatureTyping children
            for grandchild in graph.children_of(&child.id) {
                if grandchild.kind == ElementKind::FeatureTyping
                    || grandchild.kind.is_subtype_of(ElementKind::FeatureTyping)
                {
                    if let Some(type_name) = grandchild
                        .get_prop("unresolved_type")
                        .and_then(|v| v.as_str())
                    {
                        return Some(type_name.to_owned());
                    }
                }
            }
        }
    }
    None
}

/// Extract source and target endpoint strings from flow children.
///
/// The parser may create child elements representing endpoints:
/// - `EndFeatureMembership` or `FeatureMembership` children
/// - Children with `isEnd` property set
/// - Children with `unresolved_type` or `unresolved_reference` properties
///
/// The first endpoint child is treated as source, the second as target.
fn extract_endpoints_from_children(
    graph: &ModelGraph,
    flow_id: &ElementId,
) -> (Option<String>, Option<String>) {
    // Collect and sort children by span position for deterministic ordering
    // (children_of uses FxHashSet which has non-deterministic iteration)
    let mut children: Vec<_> = graph
        .children_of(flow_id)
        .filter(|child| {
            let is_end = child
                .get_prop("isEnd")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            is_end
                || child.kind == ElementKind::Feature
                || child.kind == ElementKind::ReferenceUsage
        })
        .map(|child| {
            let span_start = child.spans.first().map(|s| s.start).unwrap_or(usize::MAX);
            (child.id.clone(), span_start, child.name.clone())
        })
        .collect();

    children.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.cmp(&b.2)));

    let mut endpoints: Vec<String> = Vec::new();

    for (child_id, _, _) in &children {
        if let Some(child) = graph.get_element(child_id) {
            if let Some(name) = extract_endpoint_name(graph, child) {
                endpoints.push(name);
            }
        }
    }

    let source = endpoints.first().cloned();
    let target = endpoints.get(1).cloned();
    (source, target)
}

/// Extract an endpoint name from an element.
fn extract_endpoint_name(graph: &ModelGraph, element: &crate::Element) -> Option<String> {
    // Try direct name
    if let Some(name) = &element.name {
        return Some(name.clone());
    }

    // Try unresolved_reference property
    if let Some(ref_name) = element
        .get_prop("unresolved_reference")
        .and_then(|v| v.as_str())
    {
        return Some(ref_name.to_owned());
    }

    // Try unresolved_type property
    if let Some(type_name) = element.get_prop("unresolved_type").and_then(|v| v.as_str()) {
        return Some(type_name.to_owned());
    }

    // Try children (e.g., FeatureTyping with unresolved_type)
    for child in graph.children_of(&element.id) {
        if child.kind == ElementKind::FeatureTyping
            || child.kind.is_subtype_of(ElementKind::FeatureTyping)
        {
            if let Some(name) = child.get_prop("unresolved_type").and_then(|v| v.as_str()) {
                return Some(name.to_owned());
            }
        }
        // Also check ReferenceSubsetting
        if child.kind == ElementKind::ReferenceSubsetting || child.kind == ElementKind::Subsetting {
            if let Some(name) = child
                .get_prop("unresolved_subsettedFeature")
                .and_then(|v| v.as_str())
            {
                return Some(name.to_owned());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elaborate::elaborate;
    use crate::Element;
    use sysml_span::Span;

    #[test]
    fn derives_flow_source_target_from_children() {
        let mut graph = ModelGraph::new();

        let flow = Element::new_with_kind(ElementKind::FlowUsage).with_name("dataFlow");
        let flow_id = graph.add_element(flow);

        // Source endpoint child (span=0..10 to sort first)
        let src = Element::new_with_kind(ElementKind::Feature)
            .with_owner(flow_id.clone())
            .with_prop("isEnd", true)
            .with_name("sensor.reading")
            .with_span(Span::new("test", 0, 10));
        graph.add_element(src);

        // Target endpoint child (span=10..20 to sort second)
        let tgt = Element::new_with_kind(ElementKind::Feature)
            .with_owner(flow_id.clone())
            .with_prop("isEnd", true)
            .with_name("controller.input")
            .with_span(Span::new("test", 10, 20));
        graph.add_element(tgt);

        let report = elaborate(&mut graph);

        assert_eq!(report.flows_derived, 1);

        let elem = graph.get_element(&flow_id).unwrap();
        assert_eq!(
            elem.get_prop("source").and_then(|v| v.as_str()),
            Some("sensor.reading")
        );
        assert_eq!(
            elem.get_prop("target").and_then(|v| v.as_str()),
            Some("controller.input")
        );
    }

    #[test]
    fn does_not_overwrite_existing_props() {
        let mut graph = ModelGraph::new();

        let flow = Element::new_with_kind(ElementKind::FlowUsage)
            .with_name("existingFlow")
            .with_prop("source", "a.out")
            .with_prop("target", "b.in");
        let flow_id = graph.add_element(flow);

        let report = elaborate(&mut graph);

        assert_eq!(report.flows_derived, 0);

        let elem = graph.get_element(&flow_id).unwrap();
        assert_eq!(
            elem.get_prop("source").and_then(|v| v.as_str()),
            Some("a.out")
        );
    }

    #[test]
    fn extracts_payload_type_from_typing_child() {
        let mut graph = ModelGraph::new();

        let flow = Element::new_with_kind(ElementKind::FlowUsage).with_name("dataFlow");
        let flow_id = graph.add_element(flow);

        // FeatureTyping child representing `of Temperature`
        let typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_owner(flow_id.clone())
            .with_prop("unresolved_type", "Temperature");
        graph.add_element(typing);

        // Also add endpoints so the flow has source/target
        let src = Element::new_with_kind(ElementKind::Feature)
            .with_owner(flow_id.clone())
            .with_prop("isEnd", true)
            .with_name("sensor.reading")
            .with_span(Span::new("test", 0, 10));
        graph.add_element(src);

        let tgt = Element::new_with_kind(ElementKind::Feature)
            .with_owner(flow_id.clone())
            .with_prop("isEnd", true)
            .with_name("controller.input")
            .with_span(Span::new("test", 10, 20));
        graph.add_element(tgt);

        let report = elaborate(&mut graph);

        let elem = graph.get_element(&flow_id).unwrap();
        assert_eq!(
            elem.get_prop("payloadType").and_then(|v| v.as_str()),
            Some("Temperature")
        );
        assert_eq!(report.flows_derived, 1); // source/target also derived
    }

    #[test]
    fn extracts_payload_type_from_nested_feature() {
        let mut graph = ModelGraph::new();

        let flow = Element::new_with_kind(ElementKind::FlowUsage).with_name("typedFlow");
        let flow_id = graph.add_element(flow);

        // Payload feature (non-endpoint) with FeatureTyping grandchild
        let payload_feature = Element::new_with_kind(ElementKind::Feature)
            .with_owner(flow_id.clone())
            .with_name("payload");
        let pf_id = graph.add_element(payload_feature);

        let typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_owner(pf_id)
            .with_prop("unresolved_type", "Pressure");
        graph.add_element(typing);

        elaborate(&mut graph);

        let elem = graph.get_element(&flow_id).unwrap();
        assert_eq!(
            elem.get_prop("payloadType").and_then(|v| v.as_str()),
            Some("Pressure")
        );
    }

    #[test]
    fn does_not_overwrite_existing_payload_type() {
        let mut graph = ModelGraph::new();

        let flow = Element::new_with_kind(ElementKind::FlowUsage)
            .with_name("presetFlow")
            .with_prop("payloadType", "Voltage");
        let flow_id = graph.add_element(flow);

        let typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_owner(flow_id.clone())
            .with_prop("unresolved_type", "Current");
        graph.add_element(typing);

        elaborate(&mut graph);

        let elem = graph.get_element(&flow_id).unwrap();
        assert_eq!(
            elem.get_prop("payloadType").and_then(|v| v.as_str()),
            Some("Voltage") // Original preserved
        );
    }

    #[test]
    fn extracts_from_reference_subsetting() {
        let mut graph = ModelGraph::new();

        let flow = Element::new_with_kind(ElementKind::FlowUsage).with_name("refFlow");
        let flow_id = graph.add_element(flow);

        // Source endpoint with reference subsetting child (span=0..10 to sort first)
        let src_end = Element::new_with_kind(ElementKind::Feature)
            .with_owner(flow_id.clone())
            .with_prop("isEnd", true)
            .with_span(Span::new("test", 0, 10));
        let src_end_id = graph.add_element(src_end);

        let src_ref = Element::new_with_kind(ElementKind::ReferenceSubsetting)
            .with_owner(src_end_id)
            .with_prop("unresolved_subsettedFeature", "sensor.temp");
        graph.add_element(src_ref);

        // Target endpoint (span=10..20 to sort second)
        let tgt_end = Element::new_with_kind(ElementKind::Feature)
            .with_owner(flow_id.clone())
            .with_prop("isEnd", true)
            .with_name("controller.input")
            .with_span(Span::new("test", 10, 20));
        graph.add_element(tgt_end);

        let report = elaborate(&mut graph);

        assert_eq!(report.flows_derived, 1);

        let elem = graph.get_element(&flow_id).unwrap();
        assert_eq!(
            elem.get_prop("source").and_then(|v| v.as_str()),
            Some("sensor.temp")
        );
        assert_eq!(
            elem.get_prop("target").and_then(|v| v.as_str()),
            Some("controller.input")
        );
    }

    #[test]
    fn derives_succession_flow_usage_endpoints() {
        let mut graph = ModelGraph::new();

        let flow = Element::new_with_kind(ElementKind::SuccessionFlowUsage).with_name("succFlow");
        let flow_id = graph.add_element(flow);

        // Source endpoint child (span=0..10 to sort first)
        let src = Element::new_with_kind(ElementKind::Feature)
            .with_owner(flow_id.clone())
            .with_prop("isEnd", true)
            .with_name("step1.out")
            .with_span(Span::new("test", 0, 10));
        graph.add_element(src);

        // Target endpoint child (span=10..20 to sort second)
        let tgt = Element::new_with_kind(ElementKind::Feature)
            .with_owner(flow_id.clone())
            .with_prop("isEnd", true)
            .with_name("step2.in")
            .with_span(Span::new("test", 10, 20));
        graph.add_element(tgt);

        let report = elaborate(&mut graph);

        assert_eq!(report.flows_derived, 1);

        let elem = graph.get_element(&flow_id).unwrap();
        assert_eq!(
            elem.get_prop("source").and_then(|v| v.as_str()),
            Some("step1.out")
        );
        assert_eq!(
            elem.get_prop("target").and_then(|v| v.as_str()),
            Some("step2.in")
        );
    }

    #[test]
    fn synthesizes_flow_relationship_from_flow_usage() {
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let sensor = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("sensor")
            .with_owner(pkg_id.clone());
        let sensor_id = graph.add_element(sensor);

        let controller = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("controller")
            .with_owner(pkg_id.clone());
        let controller_id = graph.add_element(controller);

        // Flow with already-resolved source/target props
        let flow = Element::new_with_kind(ElementKind::FlowUsage)
            .with_name("dataFlow")
            .with_owner(pkg_id)
            .with_prop("source", "sensor")
            .with_prop("target", "controller");
        graph.add_element(flow);

        let report = elaborate(&mut graph);

        // Should have created a Flow relationship
        let flow_rels: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Flow)
            .collect();
        assert_eq!(flow_rels.len(), 1, "should synthesize 1 Flow relationship");
        assert_eq!(flow_rels[0].source, sensor_id);
        assert_eq!(flow_rels[0].target, controller_id);
        assert!(report.flows_derived >= 1);
    }

    #[test]
    fn synthesizes_succession_flow_relationship() {
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let step1 = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("step1")
            .with_owner(pkg_id.clone());
        let step1_id = graph.add_element(step1);

        let step2 = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("step2")
            .with_owner(pkg_id.clone());
        let step2_id = graph.add_element(step2);

        let flow = Element::new_with_kind(ElementKind::SuccessionFlowUsage)
            .with_name("succFlow")
            .with_owner(pkg_id)
            .with_prop("source", "step1")
            .with_prop("target", "step2");
        graph.add_element(flow);

        elaborate(&mut graph);

        let succ_flow_rels: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::SuccessionFlow)
            .collect();
        assert_eq!(
            succ_flow_rels.len(),
            1,
            "should synthesize SuccessionFlow relationship"
        );
        assert_eq!(succ_flow_rels[0].source, step1_id);
        assert_eq!(succ_flow_rels[0].target, step2_id);
    }

    #[test]
    fn flow_synthesis_idempotent() {
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let a = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("a")
            .with_owner(pkg_id.clone());
        graph.add_element(a);

        let b = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("b")
            .with_owner(pkg_id.clone());
        graph.add_element(b);

        let flow = Element::new_with_kind(ElementKind::FlowUsage)
            .with_owner(pkg_id)
            .with_prop("source", "a")
            .with_prop("target", "b");
        graph.add_element(flow);

        elaborate(&mut graph);
        let count_1 = graph.relationships_by_kind(&RelationshipKind::Flow).count();

        elaborate(&mut graph);
        let count_2 = graph.relationships_by_kind(&RelationshipKind::Flow).count();

        assert_eq!(
            count_1, count_2,
            "second elaborate should not add duplicates"
        );
    }
}
