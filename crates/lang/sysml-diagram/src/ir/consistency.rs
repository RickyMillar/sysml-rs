//! Scene self-consistency pass (D-B2): an edge referencing a shape that is
//! not laid out is a HARD elk `JsonImportException` in the renderer — one
//! stray edge blanks the whole view. Generators fence their own emissions,
//! but the fences have leaked repeatedly (rendered-id sets over-including
//! port features, connection endpoints resolving into unexposed subtrees…).
//! This pass makes the guarantee structural: after generation, every edge
//! either references laid-out shapes, is rerouted to the nearest laid-out
//! ownership ancestor, or is dropped (with a warning — never silently).
//!
//! "Laid out" mirrors the renderer's layout semantics exactly:
//! - root nodes always; child nodes only under a host with
//!   `expanded == Some(true)`; expanded islands' subtree nodes.
//! - ports of laid-out nodes, excluding `is_hidden` routing ports
//!   (the renderer never puts those in the elk graph).
//! - nested `DiagramChild::Edge`s participate only under an expanded host
//!   (collapsed hosts' edges never reach elk — left untouched).

use std::collections::HashSet;

use sysml_core::ModelGraph;

use super::types::{DiagramChild, DiagramEdge, DiagramIR, DiagramNode, DiagramPort};

/// Enforce edge/shape consistency on a generated scene. See module docs.
pub(crate) fn enforce_scene_consistency(scene: &mut DiagramIR, graph: &ModelGraph) {
    let laid_out = collect_laid_out_shape_ids(scene);

    // Root edges always participate in layout.
    let mut kept: Vec<DiagramEdge> = Vec::with_capacity(scene.edges.len());
    for edge in scene.edges.drain(..) {
        if let Some(e) = resolve_edge(edge, &laid_out, graph) {
            kept.push(e);
        }
    }
    scene.edges = kept;

    // Nested edges participate only under an expanded host.
    for node in &mut scene.nodes {
        resolve_nested(node, &laid_out, graph);
    }
}

fn resolve_nested(node: &mut DiagramNode, laid_out: &HashSet<String>, graph: &ModelGraph) {
    let host_expanded = node.expanded == Some(true);
    let mut resolved: Vec<DiagramChild> = Vec::with_capacity(node.children.len());
    for child in node.children.drain(..) {
        match child {
            DiagramChild::Edge(edge) if host_expanded => {
                if let Some(e) = resolve_edge(edge, laid_out, graph) {
                    resolved.push(DiagramChild::Edge(e));
                }
            }
            DiagramChild::Node(mut n) => {
                resolve_nested(&mut n, laid_out, graph);
                resolved.push(DiagramChild::Node(n));
            }
            other => resolved.push(other),
        }
    }
    node.children = resolved;
}

/// Resolve one edge against the laid-out shape set: keep as-is, reroute an
/// endpoint to its nearest laid-out ownership ancestor, or drop (None).
fn resolve_edge(
    mut edge: DiagramEdge,
    laid_out: &HashSet<String>,
    graph: &ModelGraph,
) -> Option<DiagramEdge> {
    // Port endpoints the renderer will not have: clear so the edge attaches
    // to the node endpoint instead (mirrors the renderer's own fallback).
    if let Some(p) = &edge.source_port_id {
        if !laid_out.contains(p) {
            edge.source_port_id = None;
        }
    }
    if let Some(p) = &edge.target_port_id {
        if !laid_out.contains(p) {
            edge.target_port_id = None;
        }
    }

    let rerouted_src = resolve_endpoint(&edge.source_id, laid_out, graph)?;
    let rerouted_tgt = resolve_endpoint(&edge.target_id, laid_out, graph)?;
    // A reroute that collapses the edge onto one node is meaningless visually
    // (same suppression the generators apply to collapsed-container edges).
    let was_rerouted = rerouted_src != edge.source_id || rerouted_tgt != edge.target_id;
    if was_rerouted && rerouted_src == rerouted_tgt {
        tracing::warn!(
            edge = %edge.id,
            "scene-consistency: edge collapsed to a self-loop after reroute; dropped"
        );
        return None;
    }
    edge.source_id = rerouted_src;
    edge.target_id = rerouted_tgt;
    Some(edge)
}

/// A laid-out id passes through; otherwise walk the model ownership chain to
/// the nearest laid-out ancestor. `None` (drop the edge) when nothing on the
/// chain is laid out or the id is synthetic (not a graph element).
fn resolve_endpoint(
    id: &str,
    laid_out: &HashSet<String>,
    graph: &ModelGraph,
) -> Option<String> {
    if laid_out.contains(id) {
        return Some(id.to_owned());
    }
    let mut cur = graph
        .get_element(&sysml_core::ElementId::from_string(id))
        .and_then(|e| e.owner.clone());
    let mut hops = 0usize;
    while let Some(owner_id) = cur {
        let owner_str = owner_id.to_string();
        if laid_out.contains(&owner_str) {
            return Some(owner_str);
        }
        cur = graph.get_element(&owner_id).and_then(|e| e.owner.clone());
        hops += 1;
        if hops > 64 {
            break; // ownership cycle guard
        }
    }
    tracing::warn!(
        endpoint = %id,
        "scene-consistency: edge endpoint is not laid out and has no laid-out ancestor; edge dropped"
    );
    None
}

/// Every shape id the renderer's layout will contain (see module docs).
fn collect_laid_out_shape_ids(scene: &DiagramIR) -> HashSet<String> {
    fn ports(port: &DiagramPort, ids: &mut HashSet<String>) {
        if port.is_hidden {
            return;
        }
        ids.insert(port.element_id.clone());
        for sub in &port.sub_ports {
            ports(sub, ids);
        }
    }
    fn walk(node: &DiagramNode, ids: &mut HashSet<String>) {
        ids.insert(node.element_id.clone());
        for p in &node.ports {
            ports(p, ids);
        }
        let expanded = node.expanded == Some(true);
        for child in &node.children {
            match child {
                DiagramChild::Node(n) if expanded => walk(n, ids),
                DiagramChild::Island {
                    subtree, expanded, ..
                } if *expanded => {
                    for sn in &subtree.nodes {
                        walk(sn, ids);
                    }
                }
                _ => {}
            }
        }
    }
    let mut ids = HashSet::new();
    for n in &scene.nodes {
        walk(n, &mut ids);
    }
    ids
}
