//! Connector elaboration.
//!
//! Extracts source/target endpoint names from `ConnectionUsage`, `InterfaceUsage`,
//! `AllocationUsage`, `BindingConnector`, `ConnectorAsUsage`, and
//! `BindingConnectorAsUsage` elements. The parser stores endpoints
//! as child elements (similar to flows). This pass derives the string properties
//! that downstream health diagnostics and the connector runtime expect.

use super::ElaborationReport;
use crate::{
    CanonicalKey, ElementId, ElementKind, ModelGraph, Relationship, RelationshipKind, Value,
};

/// Elaborate connectors by deriving source/target, allocation properties,
/// and synthesizing Connection/Binding/Allocate/InterfaceConnection relationships.
pub(super) fn elaborate_connectors(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    extract_connector_endpoints(graph, report);
    tag_allocation_roles(graph, report);
    synthesize_connector_relationships(graph, report);
}

/// Extract source/target from connector children using the same endpoint
/// extraction pattern as flows.
fn extract_connector_endpoints(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    let mut connector_candidate_ids = Vec::new();
    connector_candidate_ids
        .extend_from_slice(graph.element_ids_by_kind(&ElementKind::ConnectionUsage));
    connector_candidate_ids
        .extend_from_slice(graph.element_ids_by_kind(&ElementKind::InterfaceUsage));
    connector_candidate_ids
        .extend_from_slice(graph.element_ids_by_kind(&ElementKind::AllocationUsage));
    connector_candidate_ids
        .extend_from_slice(graph.element_ids_by_kind(&ElementKind::BindingConnector));
    connector_candidate_ids
        .extend_from_slice(graph.element_ids_by_kind(&ElementKind::ConnectorAsUsage));
    connector_candidate_ids
        .extend_from_slice(graph.element_ids_by_kind(&ElementKind::BindingConnectorAsUsage));

    let connector_infos: Vec<ConnectorInfo> = connector_candidate_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter(|e| {
            let has_source = e.get_prop("source").and_then(|v| v.as_str()).is_some();
            let has_target = e.get_prop("target").and_then(|v| v.as_str()).is_some();
            !has_source || !has_target
        })
        .map(|e| ConnectorInfo {
            id: e.id.clone(),
            has_source: e.get_prop("source").and_then(|v| v.as_str()).is_some(),
            has_target: e.get_prop("target").and_then(|v| v.as_str()).is_some(),
        })
        .collect();

    for info in connector_infos {
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
            report.connectors_elaborated += 1;
        }
    }
}

struct ConnectorInfo {
    id: ElementId,
    has_source: bool,
    has_target: bool,
}

/// Tag `allocatedFrom`/`allocatedTo` on AllocationUsage from source/target.
fn tag_allocation_roles(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    let alloc_ids = graph
        .element_ids_by_kind(&ElementKind::AllocationUsage)
        .to_vec();
    let to_tag: Vec<(ElementId, Option<String>, Option<String>)> = alloc_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter(|e| e.get_prop("allocatedFrom").is_none() || e.get_prop("allocatedTo").is_none())
        .map(|e| {
            let from = if e.get_prop("allocatedFrom").is_none() {
                e.get_prop("source")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            } else {
                None
            };
            let to = if e.get_prop("allocatedTo").is_none() {
                e.get_prop("target")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            } else {
                None
            };
            (e.id.clone(), from, to)
        })
        .filter(|(_, from, to)| from.is_some() || to.is_some())
        .collect();

    for (id, from, to) in to_tag {
        let mut changed = false;
        if let Some(f) = from {
            if let Some(elem) = graph.get_element_mut(&id) {
                elem.set_prop("allocatedFrom", Value::String(f));
                changed = true;
            }
        }
        if let Some(t) = to {
            if let Some(elem) = graph.get_element_mut(&id) {
                elem.set_prop("allocatedTo", Value::String(t));
                changed = true;
            }
        }
        if changed {
            report.connectors_elaborated += 1;
        }
    }
}

/// Create Relationship objects from connector elements that have resolved
/// source/target string properties.
///
/// Maps element kinds to relationship kinds:
/// - `ConnectionUsage` / `ConnectorAsUsage` → `Connection`
/// - `BindingConnector` / `BindingConnectorAsUsage` → `Binding`
/// - `AllocationUsage` → `Allocate`
/// - `InterfaceUsage` → `InterfaceConnection`
fn synthesize_connector_relationships(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    let mut synth_connector_ids = Vec::new();
    synth_connector_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::ConnectionUsage));
    synth_connector_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::InterfaceUsage));
    synth_connector_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::AllocationUsage));
    synth_connector_ids
        .extend_from_slice(graph.element_ids_by_kind(&ElementKind::BindingConnector));
    synth_connector_ids
        .extend_from_slice(graph.element_ids_by_kind(&ElementKind::ConnectorAsUsage));
    synth_connector_ids
        .extend_from_slice(graph.element_ids_by_kind(&ElementKind::BindingConnectorAsUsage));

    let conn_rels: Vec<(ElementId, ElementId, ElementId, RelationshipKind)> = synth_connector_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter_map(|e| {
            let source_name = e.get_prop("source").and_then(|v| v.as_str())?;
            let target_name = e.get_prop("target").and_then(|v| v.as_str())?;
            let source_id = super::resolve_name(graph, &e.owner, source_name)?;
            let target_id = super::resolve_name(graph, &e.owner, target_name)?;
            let rel_kind = match e.kind {
                ElementKind::BindingConnector | ElementKind::BindingConnectorAsUsage => {
                    RelationshipKind::Binding
                }
                ElementKind::AllocationUsage => RelationshipKind::Allocate,
                ElementKind::InterfaceUsage => RelationshipKind::InterfaceConnection,
                _ => RelationshipKind::Connection,
            };
            Some((e.id.clone(), source_id, target_id, rel_kind))
        })
        .collect();

    stamp_endpoint_ids(graph, report, &synth_connector_ids);

    for (origin_id, source_id, target_id, kind) in conn_rels {
        // Dedup includes origin_connector: different connectors (e.g.
        // `interface connect (molex.p, pathA.q)` and `interface connect (molex.p, pathB.q)`)
        // may resolve to the same definition-level port pair when usages share a type,
        // but each connector must produce its own relationship for the diagram layer
        // to route edges to the correct usage.
        let already_exists = graph.relationships_by_kind(&kind).any(|r| {
            r.source == source_id
                && r.target == target_id
                && r.props.get("origin_connector") == Some(&Value::Ref(origin_id.clone()))
        });

        if !already_exists {
            let src_key = CanonicalKey::root(&source_id.to_string());
            let tgt_key = CanonicalKey::root(&target_id.to_string());
            let edge_key = CanonicalKey::for_relationship(&src_key, kind.as_str(), &tgt_key, 0);
            let mut rel = Relationship::new_with_key(
                kind.clone(),
                source_id.clone(),
                target_id.clone(),
                &edge_key,
            );
            rel.props
                .insert("origin_connector".into(), Value::Ref(origin_id.clone()));
            graph.add_relationship(rel);
            report.connectors_elaborated += 1;
        }
    }
}

/// RSC-3.5a.1 (ledger L19) — additively stamp the **resolved per-usage**
/// endpoint element ids onto each origin connector/flow element as
/// `source_id` / `target_id` (`Value::Ref`), so the runtime link graph
/// (`sysml-runtime` `links.rs`) can read them directly instead of re-running a
/// def-level name-scan (`find_port_element_id`).
///
/// The ids come from [`super::resolve_endpoint_usage_id`], which reconciles the
/// three name bases (usage / def / slot) the design doc §8 L23 flags: it yields
/// **per-USAGE** identity (e.g. `controllerA` vs `controllerB`) rather than the
/// shared definition-level port that `resolve_name` returns when two usages
/// share a type. See that function for the full rule.
///
/// Additive + idempotent: skips an element that already carries `source_id` /
/// `target_id`, and only stamps when the endpoint string resolves.
pub(super) fn stamp_endpoint_ids(
    graph: &mut ModelGraph,
    report: &mut ElaborationReport,
    origin_ids: &[ElementId],
) {
    let to_stamp: Vec<(ElementId, Option<ElementId>, Option<ElementId>)> = origin_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter(|e| e.get_prop("source_id").is_none() || e.get_prop("target_id").is_none())
        .filter_map(|e| {
            let source_name = e.get_prop("source").and_then(|v| v.as_str());
            let target_name = e.get_prop("target").and_then(|v| v.as_str());
            // Nothing to do if neither endpoint string is present.
            source_name?;
            let src_id = if e.get_prop("source_id").is_none() {
                source_name.and_then(|n| super::resolve_endpoint_usage_id(graph, &e.owner, n))
            } else {
                None
            };
            let tgt_id = if e.get_prop("target_id").is_none() {
                target_name.and_then(|n| super::resolve_endpoint_usage_id(graph, &e.owner, n))
            } else {
                None
            };
            if src_id.is_none() && tgt_id.is_none() {
                return None;
            }
            Some((e.id.clone(), src_id, tgt_id))
        })
        .collect();

    for (id, src_id, tgt_id) in to_stamp {
        let mut changed = false;
        if let Some(elem) = graph.get_element_mut(&id) {
            if let Some(s) = src_id {
                elem.set_prop("source_id", Value::Ref(s));
                changed = true;
            }
            if let Some(t) = tgt_id {
                elem.set_prop("target_id", Value::Ref(t));
                changed = true;
            }
        }
        if changed {
            report.connectors_elaborated += 1;
        }
    }
}

/// Extract source and target endpoint names from connector children.
///
/// Uses the same pattern as flow endpoint extraction: sort children by span
/// position, first endpoint is source, second is target.
fn extract_endpoints_from_children(
    graph: &ModelGraph,
    connector_id: &ElementId,
) -> (Option<String>, Option<String>) {
    let mut children: Vec<_> = graph
        .children_of(connector_id)
        .filter(|child| {
            let is_end = child
                .get_prop("isEnd")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            is_end
                || child.kind == ElementKind::EndFeatureMembership
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

/// Extract an endpoint name from an element (same as flows pattern).
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

    // Try children (e.g., FeatureTyping or ReferenceSubsetting)
    for child in graph.children_of(&element.id) {
        if child.kind == ElementKind::FeatureTyping
            || child.kind.is_subtype_of(ElementKind::FeatureTyping)
        {
            if let Some(name) = child.get_prop("unresolved_type").and_then(|v| v.as_str()) {
                return Some(name.to_owned());
            }
        }
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
    fn derives_connection_source_target_from_children() {
        let mut graph = ModelGraph::new();

        let conn = Element::new_with_kind(ElementKind::ConnectionUsage).with_name("link1");
        let conn_id = graph.add_element(conn);

        let src = Element::new_with_kind(ElementKind::Feature)
            .with_owner(conn_id.clone())
            .with_prop("isEnd", true)
            .with_name("partA")
            .with_span(Span::new("test", 0, 10));
        graph.add_element(src);

        let tgt = Element::new_with_kind(ElementKind::Feature)
            .with_owner(conn_id.clone())
            .with_prop("isEnd", true)
            .with_name("partB")
            .with_span(Span::new("test", 10, 20));
        graph.add_element(tgt);

        let report = elaborate(&mut graph);

        assert!(report.connectors_elaborated >= 1);
        let elem = graph.get_element(&conn_id).unwrap();
        assert_eq!(
            elem.get_prop("source").and_then(|v| v.as_str()),
            Some("partA")
        );
        assert_eq!(
            elem.get_prop("target").and_then(|v| v.as_str()),
            Some("partB")
        );
    }

    #[test]
    fn does_not_overwrite_existing_props() {
        let mut graph = ModelGraph::new();

        let conn = Element::new_with_kind(ElementKind::ConnectionUsage)
            .with_name("existingConn")
            .with_prop("source", "a")
            .with_prop("target", "b");
        let conn_id = graph.add_element(conn);

        let report = elaborate(&mut graph);

        assert_eq!(report.connectors_elaborated, 0);
        let elem = graph.get_element(&conn_id).unwrap();
        assert_eq!(elem.get_prop("source").and_then(|v| v.as_str()), Some("a"));
    }

    #[test]
    fn tags_allocation_roles() {
        let mut graph = ModelGraph::new();

        let alloc = Element::new_with_kind(ElementKind::AllocationUsage)
            .with_name("alloc1")
            .with_prop("source", "logicalPart")
            .with_prop("target", "physicalPart");
        let alloc_id = graph.add_element(alloc);

        let report = elaborate(&mut graph);

        assert!(report.connectors_elaborated >= 1);
        let elem = graph.get_element(&alloc_id).unwrap();
        assert_eq!(
            elem.get_prop("allocatedFrom").and_then(|v| v.as_str()),
            Some("logicalPart")
        );
        assert_eq!(
            elem.get_prop("allocatedTo").and_then(|v| v.as_str()),
            Some("physicalPart")
        );
    }

    #[test]
    fn does_not_overwrite_existing_allocation_roles() {
        let mut graph = ModelGraph::new();

        let alloc = Element::new_with_kind(ElementKind::AllocationUsage)
            .with_name("alloc1")
            .with_prop("source", "a")
            .with_prop("target", "b")
            .with_prop("allocatedFrom", "original");
        let alloc_id = graph.add_element(alloc);

        elaborate(&mut graph);

        let elem = graph.get_element(&alloc_id).unwrap();
        assert_eq!(
            elem.get_prop("allocatedFrom").and_then(|v| v.as_str()),
            Some("original")
        );
        // allocatedTo should still be derived
        assert_eq!(
            elem.get_prop("allocatedTo").and_then(|v| v.as_str()),
            Some("b")
        );
    }

    #[test]
    fn extracts_interface_endpoints() {
        let mut graph = ModelGraph::new();

        let iface = Element::new_with_kind(ElementKind::InterfaceUsage).with_name("iface1");
        let iface_id = graph.add_element(iface);

        let src = Element::new_with_kind(ElementKind::Feature)
            .with_owner(iface_id.clone())
            .with_prop("isEnd", true)
            .with_name("portA")
            .with_span(Span::new("test", 0, 10));
        graph.add_element(src);

        let tgt = Element::new_with_kind(ElementKind::Feature)
            .with_owner(iface_id.clone())
            .with_prop("isEnd", true)
            .with_name("portB")
            .with_span(Span::new("test", 10, 20));
        graph.add_element(tgt);

        let report = elaborate(&mut graph);

        assert!(report.connectors_elaborated >= 1);
        let elem = graph.get_element(&iface_id).unwrap();
        assert_eq!(
            elem.get_prop("source").and_then(|v| v.as_str()),
            Some("portA")
        );
        assert_eq!(
            elem.get_prop("target").and_then(|v| v.as_str()),
            Some("portB")
        );
    }

    // === Sprint 6: ConnectorAsUsage and BindingConnectorAsUsage elaboration ===

    #[test]
    fn test_connector_as_usage_elaboration() {
        let mut graph = ModelGraph::new();

        let conn = Element::new_with_kind(ElementKind::ConnectorAsUsage).with_name("link1");
        let conn_id = graph.add_element(conn);

        let src = Element::new_with_kind(ElementKind::Feature)
            .with_owner(conn_id.clone())
            .with_prop("isEnd", true)
            .with_name("partA")
            .with_span(Span::new("test", 0, 10));
        graph.add_element(src);

        let tgt = Element::new_with_kind(ElementKind::Feature)
            .with_owner(conn_id.clone())
            .with_prop("isEnd", true)
            .with_name("partB")
            .with_span(Span::new("test", 10, 20));
        graph.add_element(tgt);

        let report = elaborate(&mut graph);

        assert!(report.connectors_elaborated >= 1);
        let elem = graph.get_element(&conn_id).unwrap();
        assert_eq!(
            elem.get_prop("source").and_then(|v| v.as_str()),
            Some("partA")
        );
        assert_eq!(
            elem.get_prop("target").and_then(|v| v.as_str()),
            Some("partB")
        );
    }

    #[test]
    fn test_binding_connector_as_usage_elaboration() {
        let mut graph = ModelGraph::new();

        // Pre-set props (as the AST builder would set them)
        let binding = Element::new_with_kind(ElementKind::BindingConnectorAsUsage)
            .with_name("bind1")
            .with_prop("source", "x")
            .with_prop("target", "y");
        let binding_id = graph.add_element(binding);

        let report = elaborate(&mut graph);

        // Already has props — should not re-elaborate
        assert_eq!(report.connectors_elaborated, 0);
        let elem = graph.get_element(&binding_id).unwrap();
        assert_eq!(elem.get_prop("source").and_then(|v| v.as_str()), Some("x"));
        assert_eq!(elem.get_prop("target").and_then(|v| v.as_str()), Some("y"));
    }

    #[test]
    fn test_binding_connector_as_usage_derives_from_children() {
        let mut graph = ModelGraph::new();

        let binding =
            Element::new_with_kind(ElementKind::BindingConnectorAsUsage).with_name("bind2");
        let binding_id = graph.add_element(binding);

        let src = Element::new_with_kind(ElementKind::Feature)
            .with_owner(binding_id.clone())
            .with_prop("isEnd", true)
            .with_name("lhs")
            .with_span(Span::new("test", 0, 10));
        graph.add_element(src);

        let tgt = Element::new_with_kind(ElementKind::Feature)
            .with_owner(binding_id.clone())
            .with_prop("isEnd", true)
            .with_name("rhs")
            .with_span(Span::new("test", 10, 20));
        graph.add_element(tgt);

        let report = elaborate(&mut graph);

        assert!(report.connectors_elaborated >= 1);
        let elem = graph.get_element(&binding_id).unwrap();
        assert_eq!(
            elem.get_prop("source").and_then(|v| v.as_str()),
            Some("lhs")
        );
        assert_eq!(
            elem.get_prop("target").and_then(|v| v.as_str()),
            Some("rhs")
        );
    }

    #[test]
    fn idempotent() {
        let mut graph = ModelGraph::new();

        let conn = Element::new_with_kind(ElementKind::ConnectionUsage).with_name("link1");
        let conn_id = graph.add_element(conn);

        let src = Element::new_with_kind(ElementKind::Feature)
            .with_owner(conn_id.clone())
            .with_prop("isEnd", true)
            .with_name("a")
            .with_span(Span::new("test", 0, 10));
        graph.add_element(src);

        let tgt = Element::new_with_kind(ElementKind::Feature)
            .with_owner(conn_id)
            .with_prop("isEnd", true)
            .with_name("b")
            .with_span(Span::new("test", 10, 20));
        graph.add_element(tgt);

        let r1 = elaborate(&mut graph);
        assert!(r1.connectors_elaborated > 0);

        let r2 = elaborate(&mut graph);
        assert_eq!(
            r2.connectors_elaborated, 0,
            "second elaborate should be no-op"
        );
    }

    #[test]
    fn synthesizes_connection_relationship() {
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let part_a = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("partA")
            .with_owner(pkg_id.clone());
        let part_a_id = graph.add_element(part_a);

        let part_b = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("partB")
            .with_owner(pkg_id.clone());
        let part_b_id = graph.add_element(part_b);

        let conn = Element::new_with_kind(ElementKind::ConnectionUsage)
            .with_name("link")
            .with_owner(pkg_id)
            .with_prop("source", "partA")
            .with_prop("target", "partB");
        graph.add_element(conn);

        elaborate(&mut graph);

        let rels: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Connection)
            .collect();
        assert_eq!(rels.len(), 1, "should synthesize Connection relationship");
        assert_eq!(rels[0].source, part_a_id);
        assert_eq!(rels[0].target, part_b_id);
    }

    #[test]
    fn synthesizes_binding_relationship() {
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let a = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("x")
            .with_owner(pkg_id.clone());
        let a_id = graph.add_element(a);

        let b = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("y")
            .with_owner(pkg_id.clone());
        let b_id = graph.add_element(b);

        let binding = Element::new_with_kind(ElementKind::BindingConnectorAsUsage)
            .with_owner(pkg_id)
            .with_prop("source", "x")
            .with_prop("target", "y");
        graph.add_element(binding);

        elaborate(&mut graph);

        let rels: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Binding)
            .collect();
        assert_eq!(rels.len(), 1, "should synthesize Binding relationship");
        assert_eq!(rels[0].source, a_id);
        assert_eq!(rels[0].target, b_id);
    }

    #[test]
    fn synthesizes_allocate_relationship() {
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let logical = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("logical")
            .with_owner(pkg_id.clone());
        let logical_id = graph.add_element(logical);

        let physical = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("physical")
            .with_owner(pkg_id.clone());
        let physical_id = graph.add_element(physical);

        let alloc = Element::new_with_kind(ElementKind::AllocationUsage)
            .with_owner(pkg_id)
            .with_prop("source", "logical")
            .with_prop("target", "physical");
        graph.add_element(alloc);

        elaborate(&mut graph);

        let rels: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Allocate)
            .collect();
        assert_eq!(rels.len(), 1, "should synthesize Allocate relationship");
        assert_eq!(rels[0].source, logical_id);
        assert_eq!(rels[0].target, physical_id);
    }

    #[test]
    fn synthesizes_interface_connection_relationship() {
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let port_a = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("portA")
            .with_owner(pkg_id.clone());
        let port_a_id = graph.add_element(port_a);

        let port_b = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("portB")
            .with_owner(pkg_id.clone());
        let port_b_id = graph.add_element(port_b);

        let iface = Element::new_with_kind(ElementKind::InterfaceUsage)
            .with_owner(pkg_id)
            .with_prop("source", "portA")
            .with_prop("target", "portB");
        graph.add_element(iface);

        elaborate(&mut graph);

        let rels: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::InterfaceConnection)
            .collect();
        assert_eq!(
            rels.len(),
            1,
            "should synthesize InterfaceConnection relationship"
        );
        assert_eq!(rels[0].source, port_a_id);
        assert_eq!(rels[0].target, port_b_id);
    }

    #[test]
    fn connector_synthesis_idempotent() {
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

        let conn = Element::new_with_kind(ElementKind::ConnectionUsage)
            .with_owner(pkg_id)
            .with_prop("source", "a")
            .with_prop("target", "b");
        graph.add_element(conn);

        elaborate(&mut graph);
        let count_1 = graph
            .relationships_by_kind(&RelationshipKind::Connection)
            .count();

        elaborate(&mut graph);
        let count_2 = graph
            .relationships_by_kind(&RelationshipKind::Connection)
            .count();

        assert_eq!(count_1, count_2, "should not duplicate relationships");
    }
}
