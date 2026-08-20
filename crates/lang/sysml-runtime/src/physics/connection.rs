//! Phase 3 — ConnectionGraph: physical port topology with junction detection.
//!
//! Transforms compiled flow connections and port registries into a physics-aware
//! directed graph. Nodes represent physical port instances, edges represent flow
//! connections, and junctions represent conservation points (Kirchhoff nodes,
//! mass-balance nodes, etc.) where multiple ports of the same domain meet on
//! a single part.

use std::collections::{HashMap, HashSet};

use sysml_core::{ElementKind, ModelGraph};
use sysml_span::Diagnostic;

use crate::flows::port::{PortDirection, PortRegistry};
use crate::links::{LinkClass, LinkEndpoint, LinkGraph};

use super::classify::{classify_port_definition, ClassificationConfidence, PortClassification};
use super::domain::{ConservationLaw, PhysicsDomainRegistry};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Index into `ConnectionGraph::nodes`.
pub type NodeId = usize;

/// Index into `ConnectionGraph::junctions`.
pub type JunctionId = usize;

/// A port instance in the physics connection graph.
#[derive(Debug, Clone)]
pub struct PhysicsPortNode {
    /// Node index within the graph.
    pub id: NodeId,
    /// Qualified path: `"owner.portName"`.
    pub qualified_path: String,
    /// Owning part instance path (e.g., `"busbar"`).
    pub owner_path: String,
    /// Port name within the owner (e.g., `"circuitOut1"`).
    pub port_name: String,
    /// Physics domain name if classified (e.g., `"electrical"`).
    pub domain: Option<&'static str>,
    /// Port direction (In, Out, InOut, Undirected).
    pub direction: PortDirection,
    /// Full port classification with per-feature breakdown, if available.
    pub classification: Option<PortClassification>,
}

/// A directed edge between two physics port nodes.
#[derive(Debug, Clone)]
pub struct PhysicsConnection {
    /// Source node index.
    pub source: NodeId,
    /// Target node index.
    pub target: NodeId,
    /// Physics domain if both endpoints share a domain.
    pub domain: Option<&'static str>,
    /// Whether this edge is currently active. Disabled edges are skipped
    /// during constraint generation and sweep solving. Used for dynamic
    /// topology changes (e.g., breaker trip disconnects a circuit).
    pub enabled: bool,
}

/// Bond graph junction type.
///
/// In bond graph theory, there are exactly two junction types:
/// - **0-junction**: common effort, flows sum to zero (KCL / parallel)
/// - **1-junction**: common flow, efforts sum to zero (KVL / series)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JunctionType {
    /// 0-junction: all bonds share the same effort. Flows are conserved (sum = 0).
    /// Physical meaning: parallel connection, Kirchhoff's Current Law.
    /// Detection: multiple same-domain ports on a single part.
    Zero,
    /// 1-junction: all bonds share the same flow. Efforts are conserved (sum = 0).
    /// Physical meaning: series connection, Kirchhoff's Voltage Law.
    /// Detection: chain of 2-port elements where flow is continuous.
    One,
}

/// A conservation junction where multiple ports of the same domain meet
/// on a single part (e.g., a busbar node for KCL).
#[derive(Debug, Clone)]
pub struct Junction {
    /// Junction index within the graph.
    pub id: JunctionId,
    /// Owning part (e.g., `"busbar"`).
    pub owner: String,
    /// Physics domain for this junction.
    pub domain: &'static str,
    /// Bond graph junction type (0-junction or 1-junction).
    pub junction_type: JunctionType,
    /// Conservation law governing this junction.
    pub conservation: ConservationLaw,
    /// Incoming port nodes: `(node_id, flow_feature_name)`.
    pub incoming: Vec<(NodeId, String)>,
    /// Outgoing port nodes: `(node_id, flow_feature_name)`.
    pub outgoing: Vec<(NodeId, String)>,
}

/// Physics-aware directed graph built from compiled flow connections.
#[derive(Debug, Clone)]
pub struct ConnectionGraph {
    /// All port nodes in the graph.
    pub nodes: Vec<PhysicsPortNode>,
    /// Directed edges (flow connections).
    pub edges: Vec<PhysicsConnection>,
    /// Conservation junctions detected from topology.
    pub junctions: Vec<Junction>,
}

// ---------------------------------------------------------------------------
// Name-based domain heuristic (fallback when Phase 2 classify is absent)
// ---------------------------------------------------------------------------

/// Classify a port definition name to a physics domain using naming heuristics.
///
/// Returns `(domain_name, flow_feature_name)` or `None`.
pub(crate) fn classify_port_def_by_name(def_name: &str) -> Option<(&'static str, &'static str)> {
    let lower = def_name.to_ascii_lowercase();
    // Electrical: explicit terms + conductor/wiring terms
    if lower.contains("electr")
        || lower.contains("circuit")
        || lower.contains("power")
        || lower.contains("phase")
        || lower.contains("neutral")
        || lower.contains("breaker")
    {
        Some(("electrical", "current"))
    } else if lower.contains("therm") || lower.contains("heat") || lower.contains("temp") {
        Some(("thermal", "heatFlow"))
    } else if lower.contains("hydraul")
        || lower.contains("water")
        || lower.contains("fluid")
        || lower.contains("pipe")
    {
        Some(("hydraulic", "massFlow"))
    } else if lower.contains("mechan") || lower.contains("force") || lower.contains("motion") {
        Some(("mechanical_translational", "force"))
    } else if lower.contains("sense") || lower.contains("sensor") || lower.contains("measure") {
        // Sensor ports are signal domain — but we still try to classify by
        // looking at what they measure. For now, return None so the nested
        // type walk or feature heuristic can take over.
        None
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

impl ConnectionGraph {
    /// Build a `ConnectionGraph` from the classified [`LinkGraph`]'s PowerBond
    /// subset (RSC-3.5f.2 / ledger L30 completion).
    ///
    /// The physics topology is the acausal effort/flow plane, so it is driven
    /// **only** by `PowerBond` links. `SignalLink` / `MessageChannel` /
    /// `Unknown` links are routed as discrete messages elsewhere and never form
    /// junctions or constitutive relations (their endpoints classify to
    /// `domain = None`, which every downstream stage — `detect_junctions`,
    /// `detect_series_junctions`, `generate_constraints_with_model` — already
    /// skips), so excluding them here is output-neutral for the DAE.
    ///
    /// Driving from the LinkGraph (rather than the `compile_flows`
    /// `FlowConnectionIR` list) additionally picks up **connector-only** power
    /// bonds — `connect a.port to b.port;` declarations with no paired
    /// `flow` — which the legacy flow-driven path silently dropped. A physical
    /// bond declared by both a flow and a connect interns as two PowerBond
    /// links over the same endpoint pair; we dedup by unordered endpoint pair
    /// so one bond yields exactly one edge (keeping the first occurrence, which
    /// is the flow-derived link's orientation when present, as the flow loop
    /// interns before the connector loop in `classify_links`).
    ///
    /// Parameters:
    /// - `link_graph` — the classified link graph (`classify_links`)
    /// - `port_registry` — port instances from `compile_ports()`
    /// - `graph` — the model graph (used for ISQ port classification)
    /// - `registry` — physics domain registry for dimension-based classification
    ///
    /// Returns the graph and any diagnostics encountered during construction.
    pub fn from_link_graph(
        link_graph: &LinkGraph,
        port_registry: &PortRegistry,
        graph: &ModelGraph,
        registry: &PhysicsDomainRegistry,
    ) -> (Self, Vec<Diagnostic>) {
        let mut diagnostics = Vec::new();
        // Map: qualified_path -> NodeId
        let mut path_to_node: HashMap<String, NodeId> = HashMap::new();
        let mut nodes: Vec<PhysicsPortNode> = Vec::new();

        // Helper: get-or-create a node for a PowerBond endpoint. Every endpoint
        // reaching here belongs to a PowerBond link, so it is ISQ/declared
        // classifiable — `classify_endpoint` (Strategy-1 ISQ only) reproduces
        // the domain the LinkGraph used to call the link a PowerBond.
        let mut ensure_node = |endpoint: &LinkEndpoint,
                               path_to_node: &mut HashMap<String, NodeId>,
                               nodes: &mut Vec<PhysicsPortNode>,
                               port_registry: &PortRegistry,
                               registry: &PhysicsDomainRegistry,
                               graph: &ModelGraph,
                               diagnostics: &mut Vec<Diagnostic>|
         -> NodeId {
            let qpath = endpoint.key();
            if let Some(&id) = path_to_node.get(&qpath) {
                return id;
            }
            let id = nodes.len();

            let (domain, _flow_feature, classification) =
                classify_endpoint(&qpath, port_registry, registry, graph, diagnostics);

            // Get direction from port registry, or default to Undirected
            let direction = port_registry
                .get(&qpath)
                .map(|p| p.direction)
                .unwrap_or(PortDirection::Undirected);

            nodes.push(PhysicsPortNode {
                id,
                qualified_path: qpath.clone(),
                owner_path: endpoint.owner.clone(),
                port_name: endpoint.port.clone(),
                domain,
                direction,
                classification,
            });
            path_to_node.insert(qpath, id);
            id
        };

        // Create nodes (dedup by qualified_path) and edges from the PowerBond
        // links, deduping multiply-declared bonds by unordered endpoint pair.
        let mut edges = Vec::new();
        let mut seen_pairs: HashSet<(String, String)> = HashSet::new();
        for &lid in link_graph.ids_of_class(LinkClass::PowerBond) {
            let Some(link) = link_graph.get(lid) else {
                continue;
            };
            let src_key = link.source.key();
            let tgt_key = link.target.key();
            let canon = if src_key <= tgt_key {
                (src_key, tgt_key)
            } else {
                (tgt_key, src_key)
            };
            if !seen_pairs.insert(canon) {
                continue;
            }

            let src_id = ensure_node(
                &link.source,
                &mut path_to_node,
                &mut nodes,
                port_registry,
                registry,
                graph,
                &mut diagnostics,
            );
            let tgt_id = ensure_node(
                &link.target,
                &mut path_to_node,
                &mut nodes,
                port_registry,
                registry,
                graph,
                &mut diagnostics,
            );

            let edge_domain = match (nodes[src_id].domain, nodes[tgt_id].domain) {
                (Some(a), Some(b)) if a == b => Some(a),
                (Some(a), None) | (None, Some(a)) => Some(a),
                _ => None,
            };

            edges.push(PhysicsConnection {
                source: src_id,
                target: tgt_id,
                domain: edge_domain,
                enabled: true,
            });
        }

        (Self::assemble_topology(nodes, edges, registry), diagnostics)
    }

    /// Run direction inference + junction detection over pre-built nodes and
    /// edges. Shared by [`Self::from_link_graph`] and the topology unit tests,
    /// which construct already-classified nodes directly (decoupling the
    /// topology algorithms from endpoint classification).
    pub(crate) fn assemble_topology(
        mut nodes: Vec<PhysicsPortNode>,
        edges: Vec<PhysicsConnection>,
        registry: &PhysicsDomainRegistry,
    ) -> Self {
        // Infer direction from topology for Undirected ports: a port that only
        // appears as an edge source is Out; only as a target is In.
        infer_directions_from_topology(&mut nodes, &edges);

        // 0-junction detection — group nodes by owner, same domain.
        let mut junctions = detect_junctions(&nodes, registry);

        // 1-junction detection — series chains of 2-port elements.
        detect_series_junctions(&nodes, &edges, registry, &mut junctions);

        ConnectionGraph {
            nodes,
            edges,
            junctions,
        }
    }

    /// Return all junctions for a specific physics domain.
    pub fn junctions_for_domain(&self, domain: &str) -> Vec<&Junction> {
        self.junctions
            .iter()
            .filter(|j| j.domain == domain)
            .collect()
    }

    /// Return a subgraph containing only nodes, edges, and junctions of the
    /// given physics domain.
    pub fn domain_subgraph(&self, domain: &str) -> ConnectionGraph {
        // Filter nodes
        let mut old_to_new: HashMap<NodeId, NodeId> = HashMap::new();
        let mut new_nodes = Vec::new();
        for node in &self.nodes {
            if node.domain == Some(domain) {
                let new_id = new_nodes.len();
                old_to_new.insert(node.id, new_id);
                let mut cloned = node.clone();
                cloned.id = new_id;
                new_nodes.push(cloned);
            }
        }

        // Filter edges (both endpoints in subgraph)
        let new_edges: Vec<PhysicsConnection> = self
            .edges
            .iter()
            .filter_map(|e| {
                let src = old_to_new.get(&e.source)?;
                let tgt = old_to_new.get(&e.target)?;
                Some(PhysicsConnection {
                    source: *src,
                    target: *tgt,
                    domain: e.domain,
                    enabled: e.enabled,
                })
            })
            .collect();

        // Filter and remap junctions
        let new_junctions: Vec<Junction> = self
            .junctions
            .iter()
            .filter(|j| j.domain == domain)
            .enumerate()
            .map(|(new_jid, j)| Junction {
                id: new_jid,
                owner: j.owner.clone(),
                domain: j.domain,
                junction_type: j.junction_type,
                conservation: j.conservation.clone(),
                incoming: j
                    .incoming
                    .iter()
                    .filter_map(|(nid, feat)| {
                        old_to_new.get(nid).map(|new_nid| (*new_nid, feat.clone()))
                    })
                    .collect(),
                outgoing: j
                    .outgoing
                    .iter()
                    .filter_map(|(nid, feat)| {
                        old_to_new.get(nid).map(|new_nid| (*new_nid, feat.clone()))
                    })
                    .collect(),
            })
            .collect();

        ConnectionGraph {
            nodes: new_nodes,
            edges: new_edges,
            junctions: new_junctions,
        }
    }

    /// Check whether the directed graph is a tree (no cycles).
    ///
    /// Uses DFS with a recursion stack to detect back-edges.
    pub fn is_tree(&self) -> bool {
        let n = self.nodes.len();
        if n == 0 {
            return true;
        }

        // Build adjacency list (skip disabled edges)
        let mut adj: Vec<Vec<NodeId>> = vec![Vec::new(); n];
        for edge in &self.edges {
            if edge.enabled {
                adj[edge.source].push(edge.target);
            }
        }

        // DFS states: 0 = unvisited, 1 = in stack, 2 = done
        let mut state = vec![0u8; n];

        for start in 0..n {
            if state[start] != 0 {
                continue;
            }
            // Iterative DFS with explicit stack
            let mut stack: Vec<(NodeId, usize)> = vec![(start, 0)];
            state[start] = 1;

            while let Some((node, idx)) = stack.last_mut() {
                if *idx < adj[*node].len() {
                    let neighbor = adj[*node][*idx];
                    *idx += 1;
                    match state[neighbor] {
                        0 => {
                            state[neighbor] = 1;
                            stack.push((neighbor, 0));
                        }
                        1 => return false, // back-edge → cycle
                        _ => {}            // cross/forward edge, ok
                    }
                } else {
                    state[*node] = 2;
                    stack.pop();
                }
            }
        }

        true
    }

    /// Disable an edge by source and target node IDs.
    ///
    /// Disabled edges are skipped during constraint generation and sweep solving.
    /// Returns true if an edge was found and disabled.
    pub fn disable_edge(&mut self, source: NodeId, target: NodeId) -> bool {
        for edge in &mut self.edges {
            if edge.source == source && edge.target == target && edge.enabled {
                edge.enabled = false;
                return true;
            }
        }
        false
    }

    /// Enable a previously disabled edge.
    ///
    /// Returns true if an edge was found and enabled.
    pub fn enable_edge(&mut self, source: NodeId, target: NodeId) -> bool {
        for edge in &mut self.edges {
            if edge.source == source && edge.target == target && !edge.enabled {
                edge.enabled = true;
                return true;
            }
        }
        false
    }

    /// Disable all edges connected to a node (both incoming and outgoing).
    ///
    /// Used when a part is disconnected (e.g., breaker trips).
    /// Returns the number of edges disabled.
    pub fn disable_node_edges(&mut self, node_id: NodeId) -> usize {
        let mut count = 0;
        for edge in &mut self.edges {
            if (edge.source == node_id || edge.target == node_id) && edge.enabled {
                edge.enabled = false;
                count += 1;
            }
        }
        count
    }

    /// Enable all edges connected to a node.
    ///
    /// Returns the number of edges enabled.
    pub fn enable_node_edges(&mut self, node_id: NodeId) -> usize {
        let mut count = 0;
        for edge in &mut self.edges {
            if (edge.source == node_id || edge.target == node_id) && !edge.enabled {
                edge.enabled = true;
                count += 1;
            }
        }
        count
    }

    /// Find a node by its qualified path.
    pub fn find_node_by_path(&self, path: &str) -> Option<NodeId> {
        self.nodes
            .iter()
            .find(|n| n.qualified_path == path)
            .map(|n| n.id)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Classify a PowerBond endpoint by its ISQ-typed port definition
/// (RSC-3.5f.2). Every endpoint reaching the ConnectionGraph belongs to a
/// `PowerBond` link, which `classify_links` only assigns when
/// [`classify_port_definition`] already resolved the endpoint to a power
/// domain (declared `@PowerPort` or inferred ISQ conjugate effort/flow). So
/// Strategy-1 (ISQ) classification here is guaranteed to reproduce that
/// domain — the legacy name-heuristic fallbacks (the old Strategies 2/3) only
/// ever fired for *non*-power endpoints that no longer reach this path and
/// were deleted with the flow-driven constructor.
///
/// The port-definition name is resolved registry-first, then by graph walk —
/// the same order `links::resolve_endpoint` uses — so the def name matches the
/// one the LinkGraph classified and the domain agrees.
///
/// Returns `(domain, flow_feature_name, Option<PortClassification>)`.
fn classify_endpoint(
    qpath: &str,
    port_registry: &PortRegistry,
    registry: &PhysicsDomainRegistry,
    graph: &ModelGraph,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Option<&'static str>, String, Option<PortClassification>) {
    let port_name = qpath.rsplit('.').next().unwrap_or(qpath);

    // Resolve the backing PortDefinition name registry-first (mirrors
    // links::resolve_endpoint), falling back to a direct graph walk.
    let def_name = port_registry
        .get(qpath)
        .and_then(|p| p.definition.clone())
        .or_else(|| find_port_definition_for_name(port_name, graph));

    if let Some(def_name) = def_name {
        let classification = classify_port_definition(&def_name, graph, registry);
        if classification.confidence != ClassificationConfidence::Unknown {
            let domain = classification.domain;
            let flow_feat = classification
                .features
                .iter()
                .find(|f| f.role == super::domain::VariableRole::Flow)
                .map(|f| f.name.clone())
                .unwrap_or_else(|| default_flow_feature_for_domain(domain));
            diagnostics.extend(classification.diagnostics.iter().cloned());
            return (domain, flow_feat, Some(classification));
        }
    }

    diagnostics.push(Diagnostic::info(format!(
        "Physics: could not graph-classify PowerBond port '{}'",
        qpath,
    )));
    (None, "flow".to_owned(), None)
}

/// Find the PortDefinition name for a port by searching the graph.
///
/// Looks for PortUsage elements with the given name that have a
/// FeatureTyping child pointing to a PortDefinition.
pub(crate) fn find_port_definition_for_name(port_name: &str, graph: &ModelGraph) -> Option<String> {
    // Find any PortUsage with this name
    for elem in graph.elements.values() {
        if elem.kind != ElementKind::PortUsage {
            continue;
        }
        if elem.name.as_deref() != Some(port_name) {
            continue;
        }

        // Check for portDefinition property (set by elaboration)
        if let Some(def) = elem.get_prop("portDefinition").and_then(|v| v.as_str()) {
            return Some(def.to_owned());
        }

        // Check FeatureTyping children for unresolved_type
        for child in graph.children_of(&elem.id) {
            if child.kind == ElementKind::FeatureTyping {
                if let Some(type_name) = child.get_prop("unresolved_type").and_then(|v| v.as_str())
                {
                    // Check if the type is a PortDefinition
                    let is_port_def = graph.elements.values().any(|e| {
                        e.kind == ElementKind::PortDefinition
                            && e.name.as_deref() == Some(type_name)
                    });
                    if is_port_def {
                        return Some(type_name.to_owned());
                    }
                }
            }
        }
    }
    None
}

/// Default flow feature name for a domain when classification doesn't provide one.
pub(crate) fn default_flow_feature_for_domain(domain: Option<&str>) -> String {
    match domain {
        Some("electrical") => "current".to_owned(),
        Some("thermal") => "heatFlow".to_owned(),
        Some("hydraulic") => "massFlow".to_owned(),
        Some("mechanical_translational") => "force".to_owned(),
        Some("mechanical_rotational") => "torque".to_owned(),
        Some("chemical") => "molar_flow".to_owned(),
        Some("luminous") => "luminous_flux".to_owned(),
        _ => "flow".to_owned(),
    }
}

/// Infer port direction from flow topology for nodes that are still Undirected.
///
/// A port that appears only as a flow source → Out.
/// A port that appears only as a flow target → In.
/// A port that appears as both → InOut.
/// This provides defense in depth when elaboration doesn't resolve direction.
fn infer_directions_from_topology(nodes: &mut [PhysicsPortNode], edges: &[PhysicsConnection]) {
    // Collect source/target appearances per node
    let mut is_source: HashSet<NodeId> = HashSet::new();
    let mut is_target: HashSet<NodeId> = HashSet::new();

    for edge in edges {
        is_source.insert(edge.source);
        is_target.insert(edge.target);
    }

    for node in nodes.iter_mut() {
        if node.direction != PortDirection::Undirected {
            continue;
        }

        let src = is_source.contains(&node.id);
        let tgt = is_target.contains(&node.id);

        node.direction = match (src, tgt) {
            (true, false) => PortDirection::Out,
            (false, true) => PortDirection::In,
            (true, true) => PortDirection::InOut,
            (false, false) => PortDirection::Undirected, // disconnected, keep undirected
        };
    }
}

/// Detect junctions: for each owner with 2+ nodes of the same domain,
/// create a Junction.
fn detect_junctions(nodes: &[PhysicsPortNode], registry: &PhysicsDomainRegistry) -> Vec<Junction> {
    // Group: (owner, domain) -> Vec<&PhysicsPortNode>
    let mut groups: HashMap<(&str, &'static str), Vec<&PhysicsPortNode>> = HashMap::new();
    for node in nodes {
        if let Some(domain) = node.domain {
            groups
                .entry((node.owner_path.as_str(), domain))
                .or_default()
                .push(node);
        }
    }

    let mut junctions = Vec::new();
    for ((owner, domain), group_nodes) in &groups {
        if group_nodes.len() < 2 {
            continue;
        }

        // Determine conservation law from the domain
        let conservation = registry
            .domains()
            .iter()
            .find(|d| d.name == *domain)
            .map(|d| d.conservation.clone())
            .unwrap_or(ConservationLaw::FlowConservation);

        let mut incoming = Vec::new();
        let mut outgoing = Vec::new();

        for node in group_nodes {
            // Determine the flow feature name from port features heuristic
            let flow_feat = determine_flow_feature(node);

            match node.direction {
                PortDirection::In => incoming.push((node.id, flow_feat)),
                PortDirection::Out => outgoing.push((node.id, flow_feat)),
                PortDirection::InOut => {
                    // InOut contributes to both sides
                    incoming.push((node.id, flow_feat.clone()));
                    outgoing.push((node.id, flow_feat));
                }
                PortDirection::Undirected => {
                    // Default to outgoing for undirected
                    outgoing.push((node.id, flow_feat));
                }
            }
        }

        let jid = junctions.len();
        junctions.push(Junction {
            id: jid,
            owner: owner.to_string(),
            domain,
            junction_type: JunctionType::Zero,
            conservation,
            incoming,
            outgoing,
        });
    }

    junctions
}

/// Detect 1-junctions (series connections) from edge topology.
///
/// A 1-junction occurs when 2-port elements (parts with exactly one In and one
/// Out port of the same domain) are connected sequentially: A.out → B.in forms
/// a series chain where all elements share the same flow.
///
/// Algorithm:
/// 1. Identify 2-port parts (exactly 1 In + 1 Out in the same domain)
/// 2. Find chains: edges connecting out-port of one 2-port to in-port of another
/// 3. For each chain of ≥2 elements, create a 1-junction
fn detect_series_junctions(
    nodes: &[PhysicsPortNode],
    edges: &[PhysicsConnection],
    registry: &PhysicsDomainRegistry,
    junctions: &mut Vec<Junction>,
) {
    // Step 1: Identify 2-port parts — (owner, domain) → (in_node_id, out_node_id)
    let mut two_port_parts: HashMap<(&str, &'static str), (NodeId, NodeId)> = HashMap::new();

    // Group nodes by (owner, domain) and check for exactly 1 In + 1 Out
    let mut owner_domain_nodes: HashMap<(&str, &'static str), Vec<&PhysicsPortNode>> =
        HashMap::new();
    for node in nodes {
        if let Some(domain) = node.domain {
            owner_domain_nodes
                .entry((node.owner_path.as_str(), domain))
                .or_default()
                .push(node);
        }
    }

    for ((owner, domain), group) in &owner_domain_nodes {
        let in_nodes: Vec<NodeId> = group
            .iter()
            .filter(|n| n.direction == PortDirection::In)
            .map(|n| n.id)
            .collect();
        let out_nodes: Vec<NodeId> = group
            .iter()
            .filter(|n| n.direction == PortDirection::Out)
            .map(|n| n.id)
            .collect();

        if in_nodes.len() == 1 && out_nodes.len() == 1 {
            two_port_parts.insert((*owner, *domain), (in_nodes[0], out_nodes[0]));
        }
    }

    if two_port_parts.is_empty() {
        return;
    }

    // Step 2: Build edge map — target_node → source_node (for chain traversal)
    // An edge src→tgt means: src.out port connects to tgt.in port
    let mut out_to_in: HashMap<NodeId, NodeId> = HashMap::new();
    for edge in edges {
        out_to_in.insert(edge.source, edge.target);
    }

    // Step 3: Find chains of 2-port elements connected in series.
    // First, find chain heads: 2-port elements whose in-port is NOT fed by
    // another 2-port element. Then traverse forward from each head.
    let mut domain_chains: HashMap<&'static str, Vec<Vec<&str>>> = HashMap::new();

    // Build reverse edge map: in_node → out_node that feeds it
    let mut in_to_out: HashMap<NodeId, NodeId> = HashMap::new();
    for edge in edges {
        in_to_out.insert(edge.target, edge.source);
    }

    // Find chain heads per domain
    let mut visited_owners: HashSet<&str> = HashSet::new();

    // Collect heads: 2-port parts where the preceding element is NOT a 2-port in same domain
    let mut heads: Vec<(&str, &'static str)> = Vec::new();
    for ((owner, domain), (in_id, _out_id)) in &two_port_parts {
        // Check if the predecessor of this element's in-port is a 2-port in same domain
        let predecessor_is_2port = in_to_out
            .get(in_id)
            .map(|&pred_out| nodes[pred_out].owner_path.as_str())
            .and_then(|pred_owner| two_port_parts.get(&(pred_owner, *domain)))
            .is_some();

        if !predecessor_is_2port {
            heads.push((*owner, *domain));
        }
    }

    // Traverse forward from each head
    for (head_owner, domain) in &heads {
        if visited_owners.contains(head_owner) {
            continue;
        }

        let mut chain = vec![*head_owner];
        visited_owners.insert(head_owner);

        let (_, mut current_out) = two_port_parts[&(*head_owner, *domain)];
        loop {
            let next_in = match out_to_in.get(&current_out) {
                Some(&target) => target,
                None => break,
            };
            let next_owner = nodes[next_in].owner_path.as_str();

            if let Some((_, next_out)) = two_port_parts.get(&(next_owner, *domain)) {
                if visited_owners.contains(next_owner) {
                    break;
                }
                chain.push(next_owner);
                visited_owners.insert(next_owner);
                current_out = *next_out;
            } else {
                break;
            }
        }

        if chain.len() >= 2 {
            domain_chains.entry(domain).or_default().push(chain);
        }
    }

    // Step 4: Create 1-junctions for each series chain
    for (domain, chains) in &domain_chains {
        let conservation = registry
            .domains()
            .iter()
            .find(|d| d.name == *domain)
            .map(|d| d.conservation.clone())
            .unwrap_or(ConservationLaw::FlowConservation);

        for chain in chains {
            let jid = junctions.len();

            // In a 1-junction, all elements share the same flow.
            // The "incoming" is the first element's in-port, "outgoing" is the last's out-port,
            // and intermediate elements contribute both.
            let mut incoming = Vec::new();
            let mut outgoing = Vec::new();

            for (i, owner) in chain.iter().enumerate() {
                if let Some((in_id, out_id)) = two_port_parts.get(&(*owner, *domain)) {
                    let flow_feat = determine_flow_feature(&nodes[*in_id]);

                    if i == 0 {
                        incoming.push((*in_id, flow_feat.clone()));
                    }
                    if i == chain.len() - 1 {
                        outgoing.push((*out_id, flow_feat));
                    } else {
                        outgoing.push((*out_id, flow_feat));
                    }
                }
            }

            // Only create junction if we have meaningful in/out
            if !incoming.is_empty() || outgoing.len() >= 2 {
                junctions.push(Junction {
                    id: jid,
                    owner: chain.join("_"),
                    domain,
                    junction_type: JunctionType::One,
                    conservation: conservation.clone(),
                    incoming,
                    outgoing,
                });
            }
        }
    }
}

/// Determine the flow feature name for a node.
///
/// Prefers the classified flow feature (from PortClassification) when available,
/// falling back to domain-based heuristics.
fn determine_flow_feature(node: &PhysicsPortNode) -> String {
    // Try to derive from classification
    if let Some(ref classification) = node.classification {
        if let Some(flow_feat) = classification
            .features
            .iter()
            .find(|f| f.role == super::domain::VariableRole::Flow)
        {
            return flow_feat.name.clone();
        }
    }

    // Fall back to domain-based heuristic
    default_flow_feature_for_domain(node.domain)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::flows::port::PortDirection;
    use crate::links::{LinkClass, LinkEndpoint, LinkGraph, LinkIR, LinkSourceKind};
    use sysml_core::ElementId;

    // ── Topology-algorithm tests ──────────────────────────────────────────
    //
    // These exercise the junction / series / subgraph algorithms directly on
    // pre-classified nodes via `assemble_topology`, decoupling them from
    // endpoint classification (covered separately by the ISQ fixtures in
    // `exchange_plane_fixture.rs`). Domains are the test's premise, set
    // explicitly — not derived from a name heuristic.

    /// Build a pre-classified physics node.
    fn enode(
        id: NodeId,
        owner: &str,
        port: &str,
        domain: Option<&'static str>,
        dir: PortDirection,
    ) -> PhysicsPortNode {
        PhysicsPortNode {
            id,
            qualified_path: format!("{owner}.{port}"),
            owner_path: owner.into(),
            port_name: port.into(),
            domain,
            direction: dir,
            classification: None,
        }
    }

    fn conn(source: NodeId, target: NodeId, domain: Option<&'static str>) -> PhysicsConnection {
        PhysicsConnection {
            source,
            target,
            domain,
            enabled: true,
        }
    }

    const ELEC: Option<&'static str> = Some("electrical");
    const HYDR: Option<&'static str> = Some("hydraulic");

    /// A PowerBond `LinkIR` over the `from`/`to` endpoint pair.
    fn power_link(elem: &str, fo: &str, fp: &str, to: &str, tp: &str) -> LinkIR {
        LinkIR {
            element_id: ElementId::from_string(elem),
            kind: LinkSourceKind::FlowUsage,
            source: LinkEndpoint {
                element_id: None,
                owner: fo.into(),
                port: fp.into(),
                resolved_registry_key: None,
            },
            target: LinkEndpoint {
                element_id: None,
                owner: to.into(),
                port: tp.into(),
                resolved_registry_key: None,
            },
            class: LinkClass::PowerBond,
            class_confidence: ClassificationConfidence::ISQTyped,
            is_succession: false,
            is_move: false,
            is_push: false,
            payload_type: None,
            source_payload_type: None,
            target_payload_type: None,
            via_interface: None,
        }
    }

    /// Test 1: source → busbar → 2 targets. Expect 1 junction on the busbar.
    #[test]
    fn busbar_junction_detected() {
        let domain_reg = PhysicsDomainRegistry::new();
        let nodes = vec![
            enode(0, "source", "powerOut", ELEC, PortDirection::Out),
            enode(1, "busbar", "powerIn", ELEC, PortDirection::In),
            enode(2, "busbar", "circuitOut1", ELEC, PortDirection::Out),
            enode(3, "busbar", "circuitOut2", ELEC, PortDirection::Out),
            enode(4, "load1", "powerIn", ELEC, PortDirection::In),
            enode(5, "load2", "powerIn", ELEC, PortDirection::In),
        ];
        let edges = vec![conn(0, 1, ELEC), conn(2, 4, ELEC), conn(3, 5, ELEC)];

        let cg = ConnectionGraph::assemble_topology(nodes, edges, &domain_reg);

        assert_eq!(cg.nodes.len(), 6, "6 unique port nodes");
        assert_eq!(cg.edges.len(), 3, "3 connections");

        // Busbar has 3 electrical ports => 1 junction
        let busbar_junctions: Vec<_> = cg
            .junctions
            .iter()
            .filter(|j| j.owner == "busbar")
            .collect();
        assert_eq!(busbar_junctions.len(), 1, "exactly 1 junction on busbar");

        let j = &busbar_junctions[0];
        assert_eq!(j.domain, "electrical");
        assert_eq!(j.incoming.len(), 1, "1 incoming (powerIn)");
        assert_eq!(j.outgoing.len(), 2, "2 outgoing (circuitOut1, circuitOut2)");
        assert_eq!(j.conservation, ConservationLaw::FlowConservation);
    }

    /// Test 2: is_tree returns true for a tree topology.
    #[test]
    fn is_tree_true_for_tree() {
        let domain_reg = PhysicsDomainRegistry::new();
        let nodes = vec![
            enode(0, "a", "out", ELEC, PortDirection::Out),
            enode(1, "b", "in", ELEC, PortDirection::In),
            enode(2, "b", "out", ELEC, PortDirection::Out),
            enode(3, "c", "in", ELEC, PortDirection::In),
        ];
        let edges = vec![conn(0, 1, ELEC), conn(2, 3, ELEC)];
        let cg = ConnectionGraph::assemble_topology(nodes, edges, &domain_reg);
        assert!(cg.is_tree(), "linear chain is a tree");
    }

    /// Test 2b: is_tree returns false for a graph with a cycle.
    #[test]
    fn is_tree_false_for_cycle() {
        // Manually construct a graph with a cycle: A→B→C→A
        let cg = ConnectionGraph {
            nodes: vec![
                PhysicsPortNode {
                    id: 0,
                    qualified_path: "a.out".into(),
                    owner_path: "a".into(),
                    port_name: "out".into(),
                    domain: None,
                    direction: PortDirection::Out,
                    classification: None,
                },
                PhysicsPortNode {
                    id: 1,
                    qualified_path: "b.out".into(),
                    owner_path: "b".into(),
                    port_name: "out".into(),
                    domain: None,
                    direction: PortDirection::Out,
                    classification: None,
                },
                PhysicsPortNode {
                    id: 2,
                    qualified_path: "c.out".into(),
                    owner_path: "c".into(),
                    port_name: "out".into(),
                    domain: None,
                    direction: PortDirection::Out,
                    classification: None,
                },
            ],
            edges: vec![
                PhysicsConnection {
                    source: 0,
                    target: 1,
                    domain: None,
                    enabled: true,
                },
                PhysicsConnection {
                    source: 1,
                    target: 2,
                    domain: None,
                    enabled: true,
                },
                PhysicsConnection {
                    source: 2,
                    target: 0,
                    domain: None,
                    enabled: true,
                }, // cycle
            ],
            junctions: vec![],
        };

        assert!(!cg.is_tree(), "cyclic graph is not a tree");
    }

    /// Test 3: junctions_for_domain filters correctly.
    #[test]
    fn junctions_for_domain_filters() {
        let domain_reg = PhysicsDomainRegistry::new();
        let nodes = vec![
            enode(0, "busbar", "in1", ELEC, PortDirection::In),
            enode(1, "busbar", "out1", ELEC, PortDirection::Out),
            enode(2, "manifold", "pipeIn", HYDR, PortDirection::In),
            enode(3, "manifold", "pipeOut", HYDR, PortDirection::Out),
        ];
        let edges = vec![conn(0, 1, ELEC), conn(2, 3, HYDR)];
        let cg = ConnectionGraph::assemble_topology(nodes, edges, &domain_reg);

        let elec = cg.junctions_for_domain("electrical");
        assert_eq!(elec.len(), 1);
        assert_eq!(elec[0].owner, "busbar");

        let hydr = cg.junctions_for_domain("hydraulic");
        assert_eq!(hydr.len(), 1);
        assert_eq!(hydr[0].owner, "manifold");

        let therm = cg.junctions_for_domain("thermal");
        assert!(therm.is_empty());
    }

    /// Test 4: domain_subgraph filters nodes, edges, junctions.
    #[test]
    fn domain_subgraph_electrical() {
        let domain_reg = PhysicsDomainRegistry::new();
        let nodes = vec![
            enode(0, "src", "eOut", ELEC, PortDirection::Out),
            enode(1, "dst", "eIn", ELEC, PortDirection::In),
            enode(2, "tank", "waterOut", HYDR, PortDirection::Out),
            enode(3, "sink", "waterIn", HYDR, PortDirection::In),
        ];
        let edges = vec![conn(0, 1, ELEC), conn(2, 3, HYDR)];
        let cg = ConnectionGraph::assemble_topology(nodes, edges, &domain_reg);

        assert_eq!(cg.nodes.len(), 4);
        assert_eq!(cg.edges.len(), 2);

        let elec_sub = cg.domain_subgraph("electrical");
        assert_eq!(elec_sub.nodes.len(), 2);
        assert_eq!(elec_sub.edges.len(), 1);
        assert_eq!(elec_sub.edges[0].source, 0);
        assert_eq!(elec_sub.edges[0].target, 1);
        // Node ids are remapped
        assert_eq!(elec_sub.nodes[0].qualified_path, "src.eOut");
        assert_eq!(elec_sub.nodes[1].qualified_path, "dst.eIn");

        // Hydraulic subgraph
        let hydr_sub = cg.domain_subgraph("hydraulic");
        assert_eq!(hydr_sub.nodes.len(), 2);
        assert_eq!(hydr_sub.edges.len(), 1);
    }

    /// Empty graph is a tree.
    #[test]
    fn empty_graph_is_tree() {
        let cg = ConnectionGraph {
            nodes: vec![],
            edges: vec![],
            junctions: vec![],
        };
        assert!(cg.is_tree());
    }

    /// Test 7: Series topology — chain of 2-port elements detects 1-junction.
    ///
    /// Circuit: source.out → r1.in, r1.out → r2.in, r2.out → r3.in, r3.out → load.in
    /// The 3 resistors (r1, r2, r3) each have 2 ports (In+Out), forming a
    /// series chain → 1-junction.
    #[test]
    fn series_chain_detects_one_junction() {
        let domain_reg = PhysicsDomainRegistry::new();
        let nodes = vec![
            enode(0, "source", "out", ELEC, PortDirection::Out),
            enode(1, "r1", "in", ELEC, PortDirection::In),
            enode(2, "r1", "out", ELEC, PortDirection::Out),
            enode(3, "r2", "in", ELEC, PortDirection::In),
            enode(4, "r2", "out", ELEC, PortDirection::Out),
            enode(5, "r3", "in", ELEC, PortDirection::In),
            enode(6, "r3", "out", ELEC, PortDirection::Out),
            enode(7, "load", "in", ELEC, PortDirection::In),
        ];
        let edges = vec![
            conn(0, 1, ELEC), // source.out → r1.in
            conn(2, 3, ELEC), // r1.out → r2.in
            conn(4, 5, ELEC), // r2.out → r3.in
            conn(6, 7, ELEC), // r3.out → load.in
        ];
        let cg = ConnectionGraph::assemble_topology(nodes, edges, &domain_reg);

        // r1, r2, r3 each have 2 electrical nodes → each gets a 0-junction;
        // the chain r1→r2→r3 should also produce a 1-junction.
        let one_junctions: Vec<_> = cg
            .junctions
            .iter()
            .filter(|j| j.junction_type == JunctionType::One)
            .collect();

        assert!(
            !one_junctions.is_empty(),
            "should detect at least one 1-junction for series chain r1→r2→r3; \
             junctions: {:?}",
            cg.junctions
                .iter()
                .map(|j| (&j.owner, j.junction_type))
                .collect::<Vec<_>>(),
        );

        let j = &one_junctions[0];
        assert_eq!(j.domain, "electrical");
        assert_eq!(j.junction_type, JunctionType::One);
    }

    // ── from_link_graph tests (PowerBond-driven construction) ─────────────

    /// Node-dedup: a shared endpoint across two PowerBond links → one node.
    /// (Classification yields domain=None on the empty graph; this exercises
    /// the structural node-dedup, not classification.)
    #[test]
    fn from_link_graph_dedups_shared_endpoint() {
        let domain_reg = PhysicsDomainRegistry::new();
        let port_reg = PortRegistry::new();
        let graph = ModelGraph::new();

        let mut lg = LinkGraph::new();
        lg.intern(power_link("l1", "hub", "out", "a", "in"));
        lg.intern(power_link("l2", "hub", "out", "b", "in"));

        let (cg, _diags) = ConnectionGraph::from_link_graph(&lg, &port_reg, &graph, &domain_reg);

        assert_eq!(cg.nodes.len(), 3, "hub.out deduped to one node");
        assert_eq!(cg.edges.len(), 2);
        assert_eq!(
            cg.edges[0].source, cg.edges[1].source,
            "both edges share hub.out"
        );
    }

    /// Edge-dedup (RSC-3.5f.2): a bond declared by both a flow and a connect
    /// interns as two PowerBond links over the same endpoint pair, but yields
    /// exactly one ConnectionGraph edge (one physical bond = one edge).
    #[test]
    fn from_link_graph_dedups_same_pair() {
        let domain_reg = PhysicsDomainRegistry::new();
        let port_reg = PortRegistry::new();
        let graph = ModelGraph::new();

        let mut lg = LinkGraph::new();
        lg.intern(power_link(
            "flow1", "supply", "powerOut", "breaker", "powerIn",
        ));
        lg.intern(power_link(
            "conn1", "supply", "powerOut", "breaker", "powerIn",
        ));

        let (cg, _diags) = ConnectionGraph::from_link_graph(&lg, &port_reg, &graph, &domain_reg);

        assert_eq!(cg.nodes.len(), 2, "two endpoint nodes");
        assert_eq!(cg.edges.len(), 1, "one edge for the doubly-declared bond");
    }

    /// Reverse-orientation dedup: a→b and b→a are the same acausal power bond.
    #[test]
    fn from_link_graph_dedups_reversed_pair() {
        let domain_reg = PhysicsDomainRegistry::new();
        let port_reg = PortRegistry::new();
        let graph = ModelGraph::new();

        let mut lg = LinkGraph::new();
        lg.intern(power_link("l1", "a", "p", "b", "q"));
        lg.intern(power_link("l2", "b", "q", "a", "p"));

        let (cg, _diags) = ConnectionGraph::from_link_graph(&lg, &port_reg, &graph, &domain_reg);

        assert_eq!(
            cg.edges.len(),
            1,
            "reversed duplicate collapses to one edge"
        );
    }

    /// Non-PowerBond links never reach the ConnectionGraph.
    #[test]
    fn from_link_graph_ignores_non_power_links() {
        let domain_reg = PhysicsDomainRegistry::new();
        let port_reg = PortRegistry::new();
        let graph = ModelGraph::new();

        let mut lg = LinkGraph::new();
        let mut sig = power_link("s1", "ctrl", "cmdOut", "motor", "cmdIn");
        sig.class = LinkClass::SignalLink;
        lg.intern(sig);
        let mut msg = power_link("m1", "a", "evtOut", "b", "evtIn");
        msg.class = LinkClass::MessageChannel;
        lg.intern(msg);

        let (cg, _diags) = ConnectionGraph::from_link_graph(&lg, &port_reg, &graph, &domain_reg);

        assert!(cg.nodes.is_empty(), "no PowerBond links → empty graph");
        assert!(cg.edges.is_empty());
    }
}
