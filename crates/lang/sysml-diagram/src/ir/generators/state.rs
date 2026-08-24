//! StateTransitionView IR generator.
//!
//! Produces `DiagramIR` for State Transition Diagrams — states, transitions,
//! initial/final pseudo-states, control nodes, and entry/do/exit compartments.
//!
//! Supports **type expansion**: when a state usage is typed by a state definition
//! (e.g. `state powerStates : MachinePowerStates`), the usage can be expanded
//! to show the definition's children inline. The `expanded_ids` set controls
//! which nodes are currently expanded.
//!
//! ## Key design decisions vs. the old the earlier state generator generator
//!
//! - **No hidden ports**: The old generator injected hidden cardinal ports (N/S/E/W)
//!   and assigned port sides. Those are rendering concerns — `render.rs` handles them.
//! - **DiagramEdge::transition()**: Transitions are expressed as `DiagramEdge` with
//!   `DiagramEdgeKind::Transition { trigger, guard }`, not raw `SEdge` structs.
//! - **Control node sizes**: Set via `DiagramNode.size`, not legacy graph model `Dimension`.
//! - **Entry/do/exit**: Expressed as `DiagramChild::Text` in the appropriate
//!   `CompartmentKind` (Entry, Do, Exit), not raw `SCompartment` construction.

use std::collections::{HashMap, HashSet};

use sysml_core::{ElementKind, ModelGraph, RelationshipKind};
use tracing::instrument;

use crate::ir::generator::{GeneratorContext, ViewGenerator};
use crate::ir::types::{DiagramIR, DiagramEdge, DiagramNode, NodeLayout, DiagramChild, DiagramButton, HeaderStyle, DiagramEdgeKind, EdgeLabelPlacement, NodeTag, CompartmentItemSource, EdgeSubLabel, EdgeSubLabelKind};
use crate::view_text;
use crate::ViewType;
use crate::visual_kind::{self as classify, CompartmentKind, VisualKind};

/// Maximum recursion depth for nested state node generation to prevent stack overflow.
const MAX_STATE_DEPTH: usize = 20;

/// Generates State Transition Diagrams.
pub struct StateTransitionViewGenerator;

impl ViewGenerator for StateTransitionViewGenerator {
    fn view_type(&self) -> ViewType {
        ViewType::StateTransition
    }

    fn elk_algorithm(&self) -> &str {
        "layered"
    }

    fn elk_direction(&self) -> Option<&str> {
        Some("DOWN")
    }

    #[instrument(skip_all)]
    fn generate(&self, ctx: &GeneratorContext) -> DiagramIR {
        let graph = ctx.graph;
        let expanded_ids = ctx.expanded_ids;

        let state_def_count = graph
            .elements_by_kind(&ElementKind::StateDefinition)
            .count();
        let usage_count = graph.elements_by_kind(&ElementKind::StateUsage).count();
        let transition_count = graph
            .elements_by_kind(&ElementKind::TransitionUsage)
            .count();
        tracing::info!(
            state_def_count,
            usage_count,
            transition_count,
            expanded_count = expanded_ids.len(),
            "StateTransitionView IR generate"
        );

        let mut ir = DiagramIR::new(ViewType::StateTransition);

        // Find all state definitions as container candidates.
        // Spec ViewFilter (4.5) is applied at top-level collection; the expose
        // fence (`in_exposed_scope`) keeps a declared `expose X` view from
        // sweeping in every StateDefinition in the merged graph (stdlib
        // included — D-B1).
        let state_defs: Vec<_> = graph
            .elements_by_kind(&ElementKind::StateDefinition)
            .filter(|e| ctx.passes_filter(e))
            .filter(|e| ctx.in_exposed_scope(e))
            .collect();

        // Collect IDs of state defs so we can identify "top-level" usages
        let state_def_ids: HashSet<_> = state_defs.iter().map(|d| d.id.clone()).collect();

        if !state_defs.is_empty() {
            // Find top-level state usages (not owned by any state def or another state usage)
            let top_level_usages: Vec<_> = graph
                .elements_by_kind(&ElementKind::StateUsage)
                .filter(|e| match &e.owner {
                    Some(owner_id) => {
                        if state_def_ids.contains(owner_id) {
                            return false;
                        }
                        // Exclude nested states (owned by another state usage)
                        if let Some(owner) = graph.get_element(owner_id) {
                            if classify::is_state_kind(&owner.kind) {
                                return false;
                            }
                        }
                        true
                    }
                    None => true,
                })
                .filter(|e| ctx.passes_filter(e))
                .filter(|e| ctx.in_exposed_scope(e))
                .collect();

            // Collect names of state definitions that are referenced by ANY typed usage
            let referenced_def_names: HashSet<String> = graph
                .elements_by_kind(&ElementKind::StateUsage)
                .filter_map(|usage| {
                    if let Some(type_name) =
                        usage.get_prop("unresolved_type").and_then(|v| v.as_str())
                    {
                        return Some(type_name.to_owned());
                    }
                    for child in graph.children_of(&usage.id) {
                        if child.kind == ElementKind::FeatureTyping
                            || child.kind.is_subtype_of(ElementKind::FeatureTyping)
                        {
                            if let Some(type_name) =
                                child.get_prop("unresolved_type").and_then(|v| v.as_str())
                            {
                                return Some(type_name.to_owned());
                            }
                        }
                    }
                    None
                })
                .collect();

            // Generate state definitions that are NOT referenced by typed usages
            for state_def in &state_defs {
                let def_name = state_def.name.as_deref().unwrap_or("");
                if !referenced_def_names.contains(def_name) {
                    let node =
                        generate_state_container(graph, state_def, expanded_ids, &mut ir);
                    ir.nodes.push(node);
                }
            }

            if !top_level_usages.is_empty() {
                for state in &top_level_usages {
                    let node = generate_state_node(graph, state, expanded_ids, &mut ir);
                    ir.nodes.push(node);
                }

                // Generate transition edges between top-level usages
                let mut top_name_index = HashMap::new();
                for usage in &top_level_usages {
                    if let Some(name) = &usage.name {
                        top_name_index.insert(name.clone(), usage.id.clone());
                    }
                }
                for sd in &state_defs {
                    if let Some(name) = &sd.name {
                        top_name_index.insert(name.clone(), sd.id.clone());
                    }
                }

                let top_transitions: Vec<_> = graph
                    .elements_by_kind(&ElementKind::TransitionUsage)
                    .filter(|e| match &e.owner {
                        Some(owner_id) => !state_def_ids.contains(owner_id),
                        None => true,
                    })
                    .collect();

                for trans in &top_transitions {
                    if let Some(edge) =
                        make_transition_edge_from_element(graph, trans, &top_name_index, None)
                    {
                        ir.edges.push(edge);
                    }
                }
            }
        } else {
            // No state definitions — show all state usages as flat nodes
            // (still inside the expose fence when the view declares one).
            let state_usages: Vec<_> = graph
                .elements_by_kind(&ElementKind::StateUsage)
                .filter(|e| ctx.in_exposed_scope(e))
                .collect();

            if !state_usages.is_empty() {
                // Add initial pseudo-state
                if let Some(initial_node) = make_initial_node(&state_usages, None) {
                    let init_id = initial_node.element_id.clone();
                    ir.nodes.push(initial_node);

                    // Wire initial to first state
                    if let Some(first_state) = find_initial_state(&state_usages) {
                        ir.edges.push(DiagramEdge::transition(
                            scoped_id(None, "init-edge"),
                            &init_id,
                            first_state.id.to_string(),
                            None,
                            None,
                        ));
                    }
                }

                for state in &state_usages {
                    let node = generate_state_node(graph, state, expanded_ids, &mut ir);
                    ir.nodes.push(node);
                }

                // Add final pseudo-states
                add_final_nodes(&state_usages, None, &mut ir);

                // Generate edges from TransitionUsage elements
                generate_transition_edges_from_elements(graph, &mut ir);
            }
        }

        // Generate edges for transition relationships (from Relationship entries).
        // DEDUP: Skip any relationship edge whose semantic signature (source, target,
        // label_text) matches an already-emitted TransitionUsage-backed edge.
        let visible_node_ids = collect_visible_node_ids(&ir);
        let emitted_ids: HashSet<String> = ir
            .nodes
            .iter()
            .map(|n| n.element_id.clone())
            .chain(collect_all_edges(&ir).into_iter().map(|e| e.id.clone()))
            .collect();
        let emitted_edge_sigs = collect_edge_signatures(&ir);

        for rel in graph.relationships_by_kind(&RelationshipKind::Transition) {
            let source_id = rel.source.to_string();
            let target_id = rel.target.to_string();
            if !visible_node_ids.contains(&source_id)
                || !visible_node_ids.contains(&target_id)
            {
                continue;
            }
            let edge_id = rel.id.to_string();
            if emitted_ids.contains(&edge_id) {
                continue;
            }

            // Build label text for dedup check
            let (trigger, guard) = extract_rel_trigger_guard(rel);
            let label_text = build_transition_label(&trigger, &guard, &None);
            let sig = (source_id.clone(), target_id.clone(), label_text);
            if emitted_edge_sigs.contains(&sig) {
                continue;
            }

            let mut edge = DiagramEdge::transition(
                edge_id, source_id, target_id, trigger, guard,
            );
            // Add trigger source annotation from relationship props
            let trigger_source = rel
                .props
                .get("trigger_port")
                .or_else(|| rel.props.get("triggerSource"))
                .map(|v| v.to_string().trim_matches('"').to_owned());
            if let Some(ref port_name) = trigger_source {
                edge.secondary_labels.push(EdgeSubLabel {
                    text: format!("[via {}]", port_name),
                    kind: EdgeSubLabelKind::TriggerSource,
                });
            }
            ir.edges.push(edge);
        }

        apply_alternating_label_placements(&mut ir.edges);
        ir
    }

    fn generate_for_owner(
        &self,
        ctx: &GeneratorContext,
        owner_id: &str,
    ) -> Option<DiagramIR> {
        let graph = ctx.graph;
        let expanded_ids = ctx.expanded_ids;
        let owner_eid = sysml_core::ElementId::from_string(owner_id);

        tracing::info!(owner_id = %owner_id, expanded_count = expanded_ids.len(), "generate_for_owner (state)");

        let mut ir = DiagramIR::new(ViewType::StateTransition);

        // Only consider states/transitions that are direct children of the owner
        let owned_states: Vec<_> = graph
            .children_of(&owner_eid)
            .filter(|c| classify::is_state_transition_child_kind(&c.kind))
            .collect();

        if owned_states.is_empty() {
            return Some(ir);
        }

        let scoped_prefix = format!("owner-{}", owner_id);

        // Add initial pseudo-state
        if let Some(initial_node) =
            make_initial_node(&owned_states, Some(scoped_prefix.as_str()))
        {
            let init_id = initial_node.element_id.clone();
            ir.nodes.push(initial_node);

            if let Some(first_state) = find_initial_state(&owned_states) {
                ir.edges.push(DiagramEdge::transition(
                    scoped_id(Some(&scoped_prefix), "init-edge"),
                    &init_id,
                    first_state.id.to_string(),
                    None,
                    None,
                ));
            }
        }

        for state_elem in &owned_states {
            let node = generate_state_node(graph, state_elem, expanded_ids, &mut ir);
            ir.nodes.push(node);
        }

        // Add final pseudo-states
        add_final_nodes(&owned_states, Some(&scoped_prefix), &mut ir);

        // Generate transition edges between owned states
        generate_transition_edges_from_elements_scoped(
            graph,
            &owner_eid,
            Some(&scoped_prefix),
            &mut ir,
        );

        // Also include Relationship-based transitions where both endpoints are visible
        let visible_node_ids = collect_visible_node_ids(&ir);
        let mut emitted_ids: HashSet<String> = ir
            .nodes
            .iter()
            .map(|n| n.element_id.clone())
            .chain(collect_all_edges(&ir).into_iter().map(|e| e.id.clone()))
            .collect();

        for rel in graph.relationships_by_kind(&RelationshipKind::Transition) {
            let source_id = rel.source.to_string();
            let target_id = rel.target.to_string();
            if !visible_node_ids.contains(&source_id)
                || !visible_node_ids.contains(&target_id)
            {
                continue;
            }
            let scoped_rel_id = format!("owner-{}/rel-{}", owner_id, rel.id);
            if !emitted_ids.insert(scoped_rel_id.clone()) {
                continue;
            }

            let (trigger, guard) = extract_rel_trigger_guard(rel);
            let mut edge = DiagramEdge::transition(
                scoped_rel_id, source_id, target_id, trigger, guard,
            );
            let trigger_source = rel
                .props
                .get("trigger_port")
                .or_else(|| rel.props.get("triggerSource"))
                .map(|v| v.to_string().trim_matches('"').to_owned());
            if let Some(ref port_name) = trigger_source {
                edge.secondary_labels.push(EdgeSubLabel {
                    text: format!("[via {}]", port_name),
                    kind: EdgeSubLabelKind::TriggerSource,
                });
            }
            ir.edges.push(edge);
        }

        apply_alternating_label_placements(&mut ir.edges);
        Some(ir)
    }
}

// ── State container (definition) ─────────────────────────────────────────

/// Generate a state definition container with its child states.
///
/// Also adds initial→first, final, and transition edges nested inside the container.
fn generate_state_container(
    graph: &ModelGraph,
    state_def: &sysml_core::Element,
    expanded_ids: &HashSet<String>,
    ir: &mut DiagramIR,
) -> DiagramNode {
    let id = state_def.id.to_string();
    let name = state_def.name.as_deref().unwrap_or("unnamed").to_owned();

    let mut node = DiagramNode::new(&id, VisualKind::State, &name);
    super::container::apply_source_metadata(&mut node, state_def, graph);
    // State containers use Free layout so ELK handles children (including edges)
    node.layout = NodeLayout::Free;
    // Auto-expand the root state container: a StateTransitionView exists to show
    // the machine's states + transitions (std lib `StandardViewDefinitions.sysml`
    // L53-62 "States with nested states … Transition usages"; spec §8.2.3.18
    // renders the composite state with substates+transitions visible, not a
    // collapsed box). Direct children are already emitted below; setting
    // `expanded` makes the renderer show them. SPEC-SILENT: depth=1 by default —
    // deeper nesting stays user-drillable via `expanded_ids` (core-steward ruling
    // 2026-06-26). The pre-existing `expanded: None` was the bug.
    node.expanded = Some(true);

    // Child states + control nodes + actions (exclude TransitionUsage — those become edges)
    let child_states: Vec<_> = graph
        .children_of(&state_def.id)
        .filter(|c| classify::is_state_transition_child_kind(&c.kind)
                    && c.get_prop("stateSubactionKind").is_none())
        .collect();

    // Track edge count before generating — new edges will be nested in the container
    let edges_before = ir.edges.len();

    // Add initial pseudo-state + edge to first state
    if let Some(initial_node) = make_initial_node(&child_states, Some(&id)) {
        let init_id = initial_node.element_id.clone();
        node.children.push(DiagramChild::Node(initial_node));

        if let Some(first_state) = find_initial_state(&child_states) {
            ir.edges.push(DiagramEdge::transition(
                scoped_id(Some(&id), "init-edge"),
                &init_id,
                first_state.id.to_string(),
                None,
                None,
            ));
        }
    }

    for child in &child_states {
        let child_node = generate_state_node_inner(graph, child, expanded_ids, 0, ir);
        node.children.push(DiagramChild::Node(child_node));
    }

    // Add final pseudo-states as children of the container + edges at IR level
    add_final_nodes_to_container(&child_states, Some(&id), &mut node, ir);

    // Generate transition edges from TransitionUsage children of this state def
    generate_transition_edges_from_elements_scoped(
        graph,
        &state_def.id,
        Some(&id),
        ir,
    );

    // Move newly-added edges into the container node (ELK requires edges at LCA level)
    let new_edges: Vec<_> = ir.edges.drain(edges_before..).collect();
    for edge in new_edges {
        node.children.push(DiagramChild::Edge(edge));
    }

    node
}

// ── State node generation ────────────────────────────────────────────────

/// Generate a state node with optional entry/do/exit actions.
fn generate_state_node(
    graph: &ModelGraph,
    element: &sysml_core::Element,
    expanded_ids: &HashSet<String>,
    ir: &mut DiagramIR,
) -> DiagramNode {
    generate_state_node_inner(graph, element, expanded_ids, 0, ir)
}

fn generate_state_node_inner(
    graph: &ModelGraph,
    element: &sysml_core::Element,
    expanded_ids: &HashSet<String>,
    depth: usize,
    ir: &mut DiagramIR,
) -> DiagramNode {
    tracing::debug!(
        element_id = %element.id,
        element_name = ?element.name,
        kind = ?element.kind,
        depth,
        "generate_state_node (IR)"
    );

    if depth >= MAX_STATE_DEPTH {
        tracing::warn!(
            element_id = %element.id,
            depth,
            "MAX_STATE_DEPTH reached, returning leaf node"
        );
        let id = element.id.to_string();
        let name = element.name.as_deref().unwrap_or("unnamed").to_owned();
        let mut node = DiagramNode::new(&id, VisualKind::State, format!("{} (max depth)", name));
        node.tags.push(NodeTag::MaxDepth);
        node.tooltip = view_text::tooltip_text(element, graph);
        return node;
    }

    // Control nodes and action usages get a simplified rendering
    if element.kind.is_control_node()
        || matches!(
            element.kind,
            ElementKind::PerformActionUsage | ElementKind::ActionUsage
        )
    {
        return generate_control_or_action_node(element, graph);
    }

    let id = element.id.to_string();
    let name = element.name.as_deref().unwrap_or("unnamed").to_owned();

    // Submachine-reference tag for state usages typed by external definitions
    let mut tags: Vec<NodeTag> = Vec::new();
    if element.kind == ElementKind::StateUsage && element.get_prop("unresolved_type").is_some() {
        tags.push(NodeTag::SubmachineRef);
    }

    // Exhibit-state tag for ExhibitStateUsage
    if element.kind == ElementKind::ExhibitStateUsage {
        tags.push(NodeTag::ExhibitState);
    }

    // Check if this state usage has a typing (references a state definition)
    let type_def = classify::find_type_definition(graph, element);

    // Check for direct nested state children
    let has_direct_nested = graph.children_of(&element.id).any(|c| {
        classify::is_state_transition_child_kind(&c.kind)
            && c.get_prop("stateSubactionKind").is_none()
    });

    // Expandable if typed OR has direct nested children
    let is_expandable = type_def.is_some() || has_direct_nested;
    let is_expanded = is_expandable && expanded_ids.contains(&id);

    if is_expanded {
        generate_expanded_state_node(
            graph,
            element,
            expanded_ids,
            depth,
            &id,
            &name,
            tags,
            type_def,
            ir,
        )
    } else {
        generate_collapsed_state_node(
            graph,
            element,
            &id,
            &name,
            tags,
            is_expandable,
            has_direct_nested,
        )
    }
}

/// Generate an expanded state node (container with child states).
fn generate_expanded_state_node(
    graph: &ModelGraph,
    element: &sysml_core::Element,
    expanded_ids: &HashSet<String>,
    depth: usize,
    id: &str,
    name: &str,
    tags: Vec<NodeTag>,
    type_def: Option<&sysml_core::Element>,
    ir: &mut DiagramIR,
) -> DiagramNode {
    // Determine children source and header text
    let (children_owner_id, header_text) = if let Some(td) = type_def {
        let type_name = td.name.as_deref().unwrap_or("unnamed");
        (td.id.clone(), format!("{} : {}", name, type_name))
    } else {
        (element.id.clone(), name.to_owned())
    };

    let mut node = DiagramNode::new(id, VisualKind::State, &header_text);
    node.expanded = Some(true);
    node.layout = NodeLayout::Free; // ELK handles children
    super::container::apply_source_metadata(&mut node, element, graph);
    node.tags = tags;

    // Add expand button
    node.buttons.push(DiagramButton::expand());

    // Populate children from the source (type def or direct owner).
    // Exclude entry/do/exit subactions — rendered separately as action labels.
    let child_states: Vec<_> = graph
        .children_of(&children_owner_id)
        .filter(|c| {
            classify::is_state_transition_child_kind(&c.kind)
                && c.get_prop("stateSubactionKind").is_none()
        })
        .collect();

    // Track edge count before generating — new edges will be nested in the container
    let edges_before = ir.edges.len();

    // Add initial pseudo-state with edge to first child state
    if let Some(initial_node) = make_initial_node(&child_states, Some(id)) {
        let init_id = initial_node.element_id.clone();
        node.children.push(DiagramChild::Node(initial_node));

        if let Some(first_state) = find_initial_state(&child_states) {
            ir.edges.push(DiagramEdge::transition(
                scoped_id(Some(id), "init-edge"),
                &init_id,
                first_state.id.to_string(),
                None,
                None,
            ));
        }
    }

    // Child states (recursively supports expansion).
    let parallel_count = child_states
        .iter()
        .filter(|c| {
            c.get_prop("isParallel")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .count();
    if parallel_count >= 2 {
        node.tags.push(NodeTag::ParallelRegions);
    }

    for child in &child_states {
        let child_node =
            generate_state_node_inner(graph, child, expanded_ids, depth + 1, ir);
        node.children.push(DiagramChild::Node(child_node));
    }

    // Final pseudo-states
    add_final_nodes_to_container(&child_states, Some(id), &mut node, ir);

    // Generate transition edges between child states of this expanded composite
    generate_transition_edges_from_elements_scoped(
        graph,
        &children_owner_id,
        Some(id),
        ir,
    );

    // Move newly-added edges into the container node (ELK requires edges at LCA level)
    let new_edges: Vec<_> = ir.edges.drain(edges_before..).collect();
    for edge in new_edges {
        node.children.push(DiagramChild::Edge(edge));
    }

    node
}

/// Generate a collapsed / leaf state node.
fn generate_collapsed_state_node(
    graph: &ModelGraph,
    element: &sysml_core::Element,
    id: &str,
    name: &str,
    tags: Vec<NodeTag>,
    is_expandable: bool,
    has_direct_nested: bool,
) -> DiagramNode {
    // Build display name with optional keyword prefix and parallel badge
    let mut display_name = String::new();
    if element.kind == ElementKind::ExhibitStateUsage {
        display_name.push_str("\u{00AB}exhibit-state\u{00BB} ");
    }
    display_name.push_str(name);
    if element
        .get_prop("isParallel")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        display_name.push_str(" \u{00AB}parallel\u{00BB}");
    }

    let stereotype = view_text::stereotype_text(&element.kind);

    let mut node = DiagramNode::new(id, VisualKind::State, &display_name);
    node.stereotype = stereotype;
    super::container::apply_source_metadata(&mut node, element, graph);
    node.tags = tags;
    node.expanded = if is_expandable { Some(false) } else { None };

    // Add expand button for expandable nodes
    if is_expandable {
        node.buttons.push(DiagramButton::expand());
    }

    // Entry/do/exit actions
    let mut subactions: Vec<(&str, String, String)> = Vec::new();
    for subaction in graph.children_of(&element.id) {
        let Some(kind_str) = subaction
            .get_prop("stateSubactionKind")
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        let action_name = subaction.name.as_deref().unwrap_or("");
        let label_text = if action_name.is_empty() {
            kind_str.to_owned()
        } else {
            format!("{} / {}", kind_str, action_name)
        };
        subactions.push((kind_str, label_text, subaction.id.to_string()));
    }
    // Sort: entry=0, do=1, exit=2
    subactions.sort_by_key(|(kind, _, _)| match *kind {
        "entry" => 0,
        "do" => 1,
        "exit" => 2,
        _ => 3,
    });
    for (kind_str, label_text, elem_id) in &subactions {
        let comp_kind = match *kind_str {
            "entry" => CompartmentKind::Entry,
            "do" => CompartmentKind::Do,
            "exit" => CompartmentKind::Exit,
            _ => continue,
        };
        node.children.push(DiagramChild::Text {
            compartment: comp_kind,
            text: label_text.clone(),
            element_id: elem_id.clone(),
            source: CompartmentItemSource::Owned,
        });
    }

    // Nested states rendered as text labels when collapsed
    if has_direct_nested {
        let nested_states: Vec<_> = graph
            .children_of(&element.id)
            .filter(|c| {
                classify::is_state_transition_child_kind(&c.kind)
                    && c.get_prop("stateSubactionKind").is_none()
            })
            .collect();
        for child in &nested_states {
            let child_name = child.name.as_deref().unwrap_or("unnamed");
            let keyword = classify::element_keyword(&child.kind);
            node.children.push(DiagramChild::Text {
                compartment: CompartmentKind::StateTransition,
                text: format!("{} {}", keyword, child_name),
                element_id: child.id.to_string(),
                source: CompartmentItemSource::Owned,
            });
        }
    }

    node
}

// ── Control / action nodes ───────────────────────────────────────────────

/// Generate a simplified node for control nodes (fork/join/decision/merge) and actions.
fn generate_control_or_action_node(
    element: &sysml_core::Element,
    graph: &ModelGraph,
) -> DiagramNode {
    let id = element.id.to_string();
    let name = element.name.as_deref().unwrap_or("unnamed").to_owned();

    let (visual_kind, size) = match element.kind {
        ElementKind::ForkNode => (VisualKind::ForkNode, (80.0, 6.0)),
        ElementKind::JoinNode => (VisualKind::JoinNode, (80.0, 6.0)),
        ElementKind::DecisionNode => (VisualKind::DecisionNode, (30.0, 30.0)),
        ElementKind::MergeNode => (VisualKind::MergeNode, (30.0, 30.0)),
        ElementKind::TerminateActionUsage => (VisualKind::TerminateNode, (20.0, 20.0)),
        ElementKind::SendActionUsage => (VisualKind::SendAction, (60.0, 40.0)),
        ElementKind::AcceptActionUsage => (VisualKind::AcceptAction, (60.0, 40.0)),
        // PerformActionUsage, ActionUsage, and other action types
        _ => (VisualKind::Action, (0.0, 0.0)), // will use auto-sizing
    };

    let is_control = element.kind.is_control_node();

    let mut node = DiagramNode::new(&id, visual_kind, if is_control { "" } else { &name });
    if is_control {
        node.header_style = HeaderStyle::None;
    }
    if size != (0.0, 0.0) {
        node.size = Some(size);
    }
    node.layout = NodeLayout::Free;
    super::container::apply_source_metadata(&mut node, element, graph);

    node
}

// ── Initial / final pseudo-states ────────────────────────────────────────

/// Create an initial pseudo-state node if states exist.
fn make_initial_node(
    states: &[&sysml_core::Element],
    id_prefix: Option<&str>,
) -> Option<DiagramNode> {
    if states.is_empty() {
        return None;
    }

    let id = scoped_id(id_prefix, "initial");
    let node = DiagramNode::new(&id, VisualKind::InitialNode, "")
        .with_header_style(HeaderStyle::None)
        .with_size(20.0, 20.0)
        .with_layout(NodeLayout::Free);

    Some(node)
}

/// Add final pseudo-state nodes for states marked as final.
fn add_final_nodes(
    states: &[&sysml_core::Element],
    id_prefix: Option<&str>,
    ir: &mut DiagramIR,
) {
    for (i, state) in states.iter().enumerate() {
        if let Some(val) = state.get_prop("isFinal") {
            if val.to_string().contains("true") {
                let final_id = scoped_id(id_prefix, &format!("final-{}", i));
                let node = DiagramNode::new(&final_id, VisualKind::FinalNode, "")
                    .with_header_style(HeaderStyle::None)
                    .with_size(20.0, 20.0)
                    .with_layout(NodeLayout::Free);
                ir.nodes.push(node);

                // Edge from state to final node
                ir.edges.push(DiagramEdge::transition(
                    scoped_id(id_prefix, &format!("final-edge-{}", i)),
                    state.id.to_string(),
                    &final_id,
                    None,
                    None,
                ));
            }
        }
    }
}

/// Add final pseudo-state nodes as children of a container node, with edges at IR level.
fn add_final_nodes_to_container(
    states: &[&sysml_core::Element],
    id_prefix: Option<&str>,
    container: &mut DiagramNode,
    ir: &mut DiagramIR,
) {
    for (i, state) in states.iter().enumerate() {
        if let Some(val) = state.get_prop("isFinal") {
            if val.to_string().contains("true") {
                let final_id = scoped_id(id_prefix, &format!("final-{}", i));
                let node = DiagramNode::new(&final_id, VisualKind::FinalNode, "")
                    .with_header_style(HeaderStyle::None)
                    .with_size(20.0, 20.0)
                    .with_layout(NodeLayout::Free);
                container.children.push(DiagramChild::Node(node));

                // Edge from state to final node
                ir.edges.push(DiagramEdge::transition(
                    scoped_id(id_prefix, &format!("final-edge-{}", i)),
                    state.id.to_string(),
                    &final_id,
                    None,
                    None,
                ));
            }
        }
    }
}

/// Find the initial state — one marked with isInitial prop, or the first state.
fn find_initial_state<'a>(states: &[&'a sysml_core::Element]) -> Option<&'a sysml_core::Element> {
    for state in states {
        if let Some(val) = state.get_prop("isInitial") {
            if val.to_string().contains("true") {
                return Some(state);
            }
        }
    }
    states.first().copied()
}

// ── Transition edge generation ───────────────────────────────────────────

/// Extract trigger and guard from a Relationship's props.
fn extract_rel_trigger_guard(
    rel: &sysml_core::Relationship,
) -> (Option<String>, Option<String>) {
    let trigger = rel
        .props
        .get("event")
        .or_else(|| rel.props.get("trigger"))
        .map(strip_value_quotes);
    let guard = rel.props.get("guard").map(strip_value_quotes);
    (trigger, guard)
}

/// Build transition label text for dedup comparison.
fn build_transition_label(
    trigger: &Option<String>,
    guard: &Option<String>,
    effect: &Option<String>,
) -> String {
    let mut parts = Vec::new();
    if let Some(t) = trigger {
        parts.push(t.clone());
    }
    if let Some(g) = guard {
        parts.push(format!("[{}]", g));
    }
    if let Some(e) = effect {
        parts.push(format!("/ {}", e));
    }
    if parts.is_empty() {
        String::new()
    } else {
        parts.join(" ")
    }
}

/// Build a name→ElementId lookup for state elements within a scope.
fn build_state_name_index(
    graph: &ModelGraph,
    scope: Option<&sysml_core::ElementId>,
) -> HashMap<String, sysml_core::ElementId> {
    let mut index = HashMap::new();
    let iter: Box<dyn Iterator<Item = &sysml_core::Element>> = match scope {
        Some(owner_id) => Box::new(graph.children_of(owner_id)),
        None => Box::new(graph.elements.values()),
    };
    for elem in iter {
        if classify::is_state_kind(&elem.kind) {
            if let Some(name) = &elem.name {
                index.insert(name.clone(), elem.id.clone());
            }
        }
    }
    index
}

/// Generate transition edges from TransitionUsage elements (top-level).
fn generate_transition_edges_from_elements(graph: &ModelGraph, ir: &mut DiagramIR) {
    let name_index = build_state_name_index(graph, None);
    let transitions: Vec<_> = graph
        .elements_by_kind(&ElementKind::TransitionUsage)
        .collect();

    for trans in &transitions {
        if let Some(edge) = make_transition_edge_from_element(graph, trans, &name_index, None) {
            ir.edges.push(edge);
        }
    }
}

/// Generate transition edges from TransitionUsage children of a specific owner.
fn generate_transition_edges_from_elements_scoped(
    graph: &ModelGraph,
    owner_id: &sysml_core::ElementId,
    id_prefix: Option<&str>,
    ir: &mut DiagramIR,
) {
    let name_index = build_state_name_index(graph, Some(owner_id));
    let transitions: Vec<_> = graph
        .children_of(owner_id)
        .filter(|c| c.kind == ElementKind::TransitionUsage)
        .collect();

    for trans in &transitions {
        if let Some(edge) = make_transition_edge_from_element(graph, trans, &name_index, id_prefix) {
            ir.edges.push(edge);
        }
    }
}

/// Create a DiagramEdge from a TransitionUsage element's source/target name props.
fn make_transition_edge_from_element(
    graph: &ModelGraph,
    trans: &sysml_core::Element,
    name_index: &HashMap<String, sysml_core::ElementId>,
    id_prefix: Option<&str>,
) -> Option<DiagramEdge> {
    let source_name = trans
        .get_prop("source")?
        .to_string()
        .trim_matches('"').to_owned();
    let target_name = trans
        .get_prop("target")?
        .to_string()
        .trim_matches('"').to_owned();

    let source_id = name_index.get(&source_name)?;
    let target_id = name_index.get(&target_name)?;

    let id = match id_prefix {
        Some(prefix) => format!("{}/trans-edge-{}", prefix, trans.id),
        None => format!("trans-edge-{}", trans.id),
    };

    // Extract trigger, guard, effect from the TransitionFeatureMembership-
    // wrapped children (one home: transition_feature_text).
    let trigger = graph.transition_feature_text(&trans.id, "trigger");
    let guard = graph.transition_feature_text(&trans.id, "guard");
    let effect = graph.transition_feature_text(&trans.id, "effect");

    // Build label for the edge. DiagramEdgeKind::Transition only has trigger + guard.
    // If there's an effect, we put it in the label field.
    let mut edge = DiagramEdge::transition(
        &id,
        source_id.to_string(),
        target_id.to_string(),
        trigger.clone(),
        guard.clone(),
    );

    // If we have an effect, compose it into the label
    if let Some(ref eff) = effect {
        edge.label = format!("/ {}", eff);
    }

    // If no trigger/guard/effect, use the transition name if available
    if trigger.is_none() && guard.is_none() && effect.is_none() {
        if let Some(n) = &trans.name {
            edge.label = n.clone();
        }
    }

    // Trigger source annotation: if the transition has a trigger_port or
    // triggerSource property, add a secondary label showing the source port.
    let trigger_source = trans
        .get_prop("trigger_port")
        .or_else(|| trans.get_prop("triggerSource"))
        .map(|v| v.to_string().trim_matches('"').to_owned());
    if let Some(ref port_name) = trigger_source {
        edge.secondary_labels.push(EdgeSubLabel {
            text: format!("[via {}]", port_name),
            kind: EdgeSubLabelKind::TriggerSource,
        });
    }

    Some(edge)
}

/// Post-process transition edges to alternate label placement when multiple
/// edges share the same source/target pair. This prevents label overlap.
#[allow(clippy::indexing_slicing)] // pos_idx bounded by `% positions.len()`
fn apply_alternating_label_placements(edges: &mut [DiagramEdge]) {
    // Two strategies:
    // 1. Multi-edge pairs: same (source, target) → spread along edge path
    // 2. Node convergence: multiple edges share a common endpoint → spread from center

    // --- Strategy 1: Multi-edge pairs ---
    let mut pair_counts: HashMap<(String, String), usize> = HashMap::new();
    for edge in edges.iter() {
        if matches!(edge.kind, DiagramEdgeKind::Transition { .. }) {
            let key = normalize_pair(&edge.source_id, &edge.target_id);
            *pair_counts.entry(key).or_insert(0) += 1;
        }
    }
    let multi_pairs: HashSet<(String, String)> = pair_counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(pair, _)| pair)
        .collect();

    let positions = [0.3, 0.5, 0.7, 0.4, 0.6];
    let offsets = [12.0, 16.0, 24.0, 20.0, 28.0];

    let mut pair_index: HashMap<(String, String), usize> = HashMap::new();
    for edge in edges.iter_mut() {
        if matches!(edge.kind, DiagramEdgeKind::Transition { .. }) {
            let key = normalize_pair(&edge.source_id, &edge.target_id);
            if multi_pairs.contains(&key) {
                let idx = pair_index.entry(key).or_insert(0);
                let pos_idx = *idx % positions.len();
                edge.label_placement = EdgeLabelPlacement {
                    position: positions[pos_idx],
                    side: if (*idx).is_multiple_of(2) { "left" } else { "right" }.to_owned(),
                    offset: Some(offsets[pos_idx]),
                    rotate: false,
                };
                *idx += 1;
            }
        }
    }

    // --- Strategy 2: Node convergence ---
    // Count edges per node (as source or target). If a node has 3+ edges,
    // spread the labels of those edges to reduce clustering near the node.
    let mut node_edge_count: HashMap<String, usize> = HashMap::new();
    for edge in edges.iter() {
        if matches!(edge.kind, DiagramEdgeKind::Transition { .. }) {
            *node_edge_count.entry(edge.source_id.clone()).or_insert(0) += 1;
            *node_edge_count.entry(edge.target_id.clone()).or_insert(0) += 1;
        }
    }
    let congested_nodes: HashSet<String> = node_edge_count
        .into_iter()
        .filter(|(_, count)| *count >= 3)
        .map(|(id, _)| id)
        .collect();

    if congested_nodes.is_empty() {
        return;
    }

    // For edges touching congested nodes (that weren't already handled by Strategy 1),
    // shift the label position away from the congested endpoint.
    let mut node_label_idx: HashMap<String, usize> = HashMap::new();
    let spread_positions = [0.35, 0.45, 0.55, 0.65, 0.40, 0.60];

    for edge in edges.iter_mut() {
        if !matches!(edge.kind, DiagramEdgeKind::Transition { .. }) {
            continue;
        }
        // Skip edges already handled by Strategy 1
        let key = normalize_pair(&edge.source_id, &edge.target_id);
        if multi_pairs.contains(&key) {
            continue;
        }

        let source_congested = congested_nodes.contains(&edge.source_id);
        let target_congested = congested_nodes.contains(&edge.target_id);

        if source_congested || target_congested {
            // Use the congested node's index to pick a spread position
            let congested_id = if target_congested {
                &edge.target_id
            } else {
                &edge.source_id
            };
            let idx = node_label_idx.entry(congested_id.clone()).or_insert(0);
            let pos_idx = *idx % spread_positions.len();

            // Shift label away from the congested end
            let position = if target_congested {
                // Target is congested — move label toward source
                spread_positions[pos_idx] * 0.7
            } else {
                // Source is congested — move label toward target
                1.0 - spread_positions[pos_idx] * 0.7
            };
            let side = if pos_idx % 2 == 0 { "left" } else { "right" };

            edge.label_placement = EdgeLabelPlacement {
                position,
                side: side.to_owned(),
                offset: Some(8.0 + (pos_idx as f64) * 4.0),
                rotate: false,
            };
            *idx += 1;
        }
    }
}

/// Normalize a pair of IDs so (A,B) and (B,A) produce the same key.
fn normalize_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_owned(), b.to_owned())
    } else {
        (b.to_owned(), a.to_owned())
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn scoped_id(id_prefix: Option<&str>, base_id: &str) -> String {
    match id_prefix {
        Some(prefix) => format!("{}/{}", prefix, base_id),
        None => base_id.to_owned(),
    }
}

/// Strip surrounding quotes from a Value's Display output.
fn strip_value_quotes(v: &impl std::fmt::Display) -> String {
    v.to_string().trim_matches('"').to_owned()
}

/// Collect all visible node IDs from the IR (nodes + children recursively).
fn collect_visible_node_ids(ir: &DiagramIR) -> HashSet<String> {
    let mut ids = HashSet::new();
    for node in &ir.nodes {
        collect_node_ids_recursive(node, &mut ids);
    }
    ids
}

fn collect_node_ids_recursive(node: &DiagramNode, ids: &mut HashSet<String>) {
    ids.insert(node.element_id.clone());
    for child in &node.children {
        if let DiagramChild::Node(child_node) = child {
            collect_node_ids_recursive(child_node, ids);
        }
    }
}

/// Collect semantic edge signatures for dedup (includes edges nested in containers).
fn collect_edge_signatures(ir: &DiagramIR) -> HashSet<(String, String, String)> {
    let mut sigs = HashSet::new();
    let all_edges = collect_all_edges(ir);
    for edge in all_edges {
        let label = match &edge.kind {
            DiagramEdgeKind::Transition { trigger, guard } => {
                build_transition_label(trigger, guard, &None)
            }
            _ => edge.label.clone(),
        };
        // Combine with explicit label field if present
        let full_label = if !edge.label.is_empty() && !label.is_empty() {
            format!("{} {}", label, edge.label)
        } else if !edge.label.is_empty() {
            edge.label.clone()
        } else {
            label
        };
        sigs.insert((edge.source_id.clone(), edge.target_id.clone(), full_label));
    }
    sigs
}

/// Collect all edges from the IR, including those nested inside container nodes.
fn collect_all_edges(ir: &DiagramIR) -> Vec<&DiagramEdge> {
    let mut edges: Vec<&DiagramEdge> = ir.edges.iter().collect();
    for node in &ir.nodes {
        collect_nested_edges(node, &mut edges);
    }
    edges
}

fn collect_nested_edges<'a>(node: &'a DiagramNode, edges: &mut Vec<&'a DiagramEdge>) {
    for child in &node.children {
        match child {
            DiagramChild::Edge(e) => edges.push(e),
            DiagramChild::Node(n) => collect_nested_edges(n, edges),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_core::{Element, Relationship};

    fn make_ctx<'a>(graph: &'a ModelGraph, expanded_ids: &'a HashSet<String>) -> GeneratorContext<'a> {
        GeneratorContext::new(graph, expanded_ids)
    }

    /// Collect all edges from an IR — both root-level and nested inside container nodes.
    fn all_edges(ir: &DiagramIR) -> Vec<&DiagramEdge> {
        collect_all_edges(ir)
    }

    // ── Basic state generation ───────────────────────────────────────────

    #[test]
    fn empty_graph_produces_empty_ir() {
        let graph = ModelGraph::new();
        let expanded = HashSet::new();
        let gen = StateTransitionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &expanded));
        assert!(ir.nodes.is_empty());
        assert!(ir.edges.is_empty());
        assert_eq!(ir.view_type, ViewType::StateTransition);
    }

    #[test]
    fn state_def_with_usages_produces_container() {
        let mut graph = ModelGraph::new();
        let state_def =
            Element::new_with_kind(ElementKind::StateDefinition).with_name("TrafficLight");
        let def_id = graph.add_element(state_def);

        let red = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Red")
            .with_owner(def_id.clone());
        let red_id = graph.add_element(red);

        let green = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Green")
            .with_owner(def_id.clone());
        let green_id = graph.add_element(green);

        let transition = Relationship::new(RelationshipKind::Transition, red_id, green_id);
        graph.add_relationship(transition);

        let gen = StateTransitionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &HashSet::new()));

        // Should have the container node
        assert!(!ir.nodes.is_empty());
        let container = &ir.nodes[0];
        assert_eq!(container.visual_kind, VisualKind::State);
        assert_eq!(container.name, "TrafficLight");

        // The root state container auto-expands (depth=1): a StateTransitionView
        // must show its states + transitions, not a collapsed box (core-steward
        // ruling 2026-06-26; std lib StandardViewDefinitions + spec §8.2.3.18).
        assert_eq!(
            container.expanded,
            Some(true),
            "root state container should auto-expand to show substates/transitions"
        );

        // Container should have child nodes (initial + Red + Green)
        let child_nodes: Vec<_> = container
            .children
            .iter()
            .filter(|c| matches!(c, DiagramChild::Node(_)))
            .collect();
        assert!(
            child_nodes.len() >= 2,
            "expected at least 2 child nodes (states), got {}",
            child_nodes.len()
        );

        // Should have transition edges (nested inside container node)
        let container_edges: Vec<_> = container
            .children
            .iter()
            .filter(|c| matches!(c, DiagramChild::Edge(_)))
            .collect();
        assert!(
            !container_edges.is_empty(),
            "expected transition edges nested in container"
        );
    }

    #[test]
    fn flat_usages_without_definition() {
        let mut graph = ModelGraph::new();

        let idle = Element::new_with_kind(ElementKind::StateUsage).with_name("Idle");
        let idle_id = graph.add_element(idle);

        let running = Element::new_with_kind(ElementKind::StateUsage).with_name("Running");
        let running_id = graph.add_element(running);

        let t = Relationship::new(RelationshipKind::Transition, idle_id, running_id);
        graph.add_relationship(t);

        let gen = StateTransitionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &HashSet::new()));

        // Should have initial + Idle + Running
        assert!(ir.nodes.len() >= 3, "expected at least 3 nodes, got {}", ir.nodes.len());

        // Should have initial node
        let has_initial = ir
            .nodes
            .iter()
            .any(|n| n.visual_kind == VisualKind::InitialNode);
        assert!(has_initial, "expected initial node");

        // Should have transition edges
        assert!(!ir.edges.is_empty(), "expected transition edges");
    }

    // ── Control nodes ────────────────────────────────────────────────────

    #[test]
    fn fork_join_nodes() {
        let mut graph = ModelGraph::new();
        let state_def =
            Element::new_with_kind(ElementKind::StateDefinition).with_name("Workflow");
        let def_id = graph.add_element(state_def);

        let fork = Element::new_with_kind(ElementKind::ForkNode)
            .with_name("f1")
            .with_owner(def_id.clone());
        graph.add_element(fork);

        let join = Element::new_with_kind(ElementKind::JoinNode)
            .with_name("j1")
            .with_owner(def_id);
        graph.add_element(join);

        let gen = StateTransitionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &HashSet::new()));

        let container = &ir.nodes[0];
        let child_kinds: Vec<_> = container
            .children
            .iter()
            .filter_map(|c| match c {
                DiagramChild::Node(n) => Some(n.visual_kind),
                _ => None,
            })
            .collect();
        assert!(
            child_kinds.contains(&VisualKind::ForkNode),
            "expected ForkNode in children"
        );
        assert!(
            child_kinds.contains(&VisualKind::JoinNode),
            "expected JoinNode in children"
        );
    }

    #[test]
    fn decision_merge_nodes() {
        let mut graph = ModelGraph::new();
        let state_def =
            Element::new_with_kind(ElementKind::StateDefinition).with_name("Workflow");
        let def_id = graph.add_element(state_def);

        let decision = Element::new_with_kind(ElementKind::DecisionNode)
            .with_name("d1")
            .with_owner(def_id.clone());
        graph.add_element(decision);

        let merge = Element::new_with_kind(ElementKind::MergeNode)
            .with_name("m1")
            .with_owner(def_id);
        graph.add_element(merge);

        let gen = StateTransitionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &HashSet::new()));

        let container = &ir.nodes[0];
        let child_kinds: Vec<_> = container
            .children
            .iter()
            .filter_map(|c| match c {
                DiagramChild::Node(n) => Some(n.visual_kind),
                _ => None,
            })
            .collect();
        assert!(
            child_kinds.contains(&VisualKind::DecisionNode),
            "expected DecisionNode"
        );
        assert!(
            child_kinds.contains(&VisualKind::MergeNode),
            "expected MergeNode"
        );
    }

    #[test]
    fn control_node_sizes() {
        let graph = ModelGraph::new();
        let fork = Element::new_with_kind(ElementKind::ForkNode).with_name("f1");
        let node = generate_control_or_action_node(&fork, &graph);
        assert_eq!(node.size, Some((80.0, 6.0)));
        assert_eq!(node.visual_kind, VisualKind::ForkNode);

        let join = Element::new_with_kind(ElementKind::JoinNode).with_name("j1");
        let node = generate_control_or_action_node(&join, &graph);
        assert_eq!(node.size, Some((80.0, 6.0)));

        let decision = Element::new_with_kind(ElementKind::DecisionNode).with_name("d1");
        let node = generate_control_or_action_node(&decision, &graph);
        assert_eq!(node.size, Some((30.0, 30.0)));

        let merge = Element::new_with_kind(ElementKind::MergeNode).with_name("m1");
        let node = generate_control_or_action_node(&merge, &graph);
        assert_eq!(node.size, Some((30.0, 30.0)));
    }

    #[test]
    fn control_nodes_have_no_header() {
        let graph = ModelGraph::new();
        let fork = Element::new_with_kind(ElementKind::ForkNode).with_name("f1");
        let node = generate_control_or_action_node(&fork, &graph);
        assert_eq!(node.header_style, HeaderStyle::None);
        assert!(node.name.is_empty(), "control nodes should have empty name");
    }

    #[test]
    fn action_nodes_have_name() {
        let graph = ModelGraph::new();
        let perform = Element::new_with_kind(ElementKind::PerformActionUsage).with_name("doWork");
        let node = generate_control_or_action_node(&perform, &graph);
        assert_eq!(node.name, "doWork");
        assert_eq!(node.visual_kind, VisualKind::Action);
    }

    // ── Entry/Do/Exit compartments ───────────────────────────────────────

    #[test]
    fn entry_do_exit_actions() {
        let mut graph = ModelGraph::new();

        let state_def =
            Element::new_with_kind(ElementKind::StateDefinition).with_name("Machine");
        let def_id = graph.add_element(state_def);

        let state = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("active")
            .with_owner(def_id);
        let state_id = graph.add_element(state);

        let entry = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("TurnOn")
            .with_owner(state_id.clone())
            .with_prop("stateSubactionKind", "entry");
        graph.add_element(entry);

        let do_action = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("Monitor")
            .with_owner(state_id.clone())
            .with_prop("stateSubactionKind", "do");
        graph.add_element(do_action);

        let exit = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("TurnOff")
            .with_owner(state_id)
            .with_prop("stateSubactionKind", "exit");
        graph.add_element(exit);

        let gen = StateTransitionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &HashSet::new()));

        // Find the "active" state node (it's a child of the container)
        let container = &ir.nodes[0];
        let active_node = container
            .children
            .iter()
            .find_map(|c| match c {
                DiagramChild::Node(n) if n.name.contains("active") => Some(n),
                _ => None,
            })
            .expect("should find 'active' state node");

        // Check for entry/do/exit text children
        let text_children: Vec<_> = active_node
            .children
            .iter()
            .filter_map(|c| match c {
                DiagramChild::Text {
                    compartment, text, ..
                } => Some((*compartment, text.as_str())),
                _ => None,
            })
            .collect();

        assert!(
            text_children.iter().any(|(k, t)| *k == CompartmentKind::Entry && t.contains("entry")),
            "expected entry text, got {:?}",
            text_children
        );
        assert!(
            text_children.iter().any(|(k, t)| *k == CompartmentKind::Do && t.contains("do")),
            "expected do text, got {:?}",
            text_children
        );
        assert!(
            text_children.iter().any(|(k, t)| *k == CompartmentKind::Exit && t.contains("exit")),
            "expected exit text, got {:?}",
            text_children
        );
    }

    #[test]
    fn entry_action_order_is_entry_do_exit() {
        let mut graph = ModelGraph::new();

        let state = Element::new_with_kind(ElementKind::StateUsage).with_name("s1");
        let state_id = graph.add_element(state);

        // Add in reverse order
        let exit = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("ExitAction")
            .with_owner(state_id.clone())
            .with_prop("stateSubactionKind", "exit");
        graph.add_element(exit);

        let entry = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("EntryAction")
            .with_owner(state_id.clone())
            .with_prop("stateSubactionKind", "entry");
        graph.add_element(entry);

        let do_action = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("DoAction")
            .with_owner(state_id)
            .with_prop("stateSubactionKind", "do");
        graph.add_element(do_action);

        let gen = StateTransitionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &HashSet::new()));

        // Find s1 node
        let s1 = ir
            .nodes
            .iter()
            .find(|n| n.name.contains("s1"))
            .expect("should find s1");

        let compartments: Vec<CompartmentKind> = s1
            .children
            .iter()
            .filter_map(|c| match c {
                DiagramChild::Text { compartment, .. } => Some(*compartment),
                _ => None,
            })
            .collect();

        assert_eq!(
            compartments,
            vec![CompartmentKind::Entry, CompartmentKind::Do, CompartmentKind::Exit],
            "subactions should be sorted entry, do, exit"
        );
    }

    // ── Typed expansion ──────────────────────────────────────────────────

    #[test]
    fn typed_usage_expandable() {
        let mut graph = ModelGraph::new();

        let state_def =
            Element::new_with_kind(ElementKind::StateDefinition).with_name("MachinePowerStates");
        let def_id = graph.add_element(state_def);

        let off = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("off")
            .with_owner(def_id.clone());
        graph.add_element(off);

        let on = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("on")
            .with_owner(def_id);
        graph.add_element(on);

        // Typed usage at package level
        let usage = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("powerStates")
            .with_prop("unresolved_type", "MachinePowerStates");
        let usage_id = graph.add_element(usage);

        let gen = StateTransitionViewGenerator;

        // Collapsed: should be expandable but not expanded
        let ir = gen.generate(&make_ctx(&graph, &HashSet::new()));
        let usage_node = ir
            .nodes
            .iter()
            .find(|n| n.name.contains("powerStates"))
            .expect("should find powerStates");
        assert_eq!(usage_node.expanded, Some(false));

        // Expanded: should show children from MachinePowerStates
        let mut expanded = HashSet::new();
        expanded.insert(usage_id.to_string());
        let ir = gen.generate(&make_ctx(&graph, &expanded));
        let usage_node = ir
            .nodes
            .iter()
            .find(|n| n.name.contains("powerStates"))
            .expect("should find powerStates");
        assert_eq!(usage_node.expanded, Some(true));
        assert!(
            usage_node.name.contains("MachinePowerStates"),
            "expanded header should show type name, got: {}",
            usage_node.name
        );

        // Should have child nodes
        let child_count = usage_node
            .children
            .iter()
            .filter(|c| matches!(c, DiagramChild::Node(_)))
            .count();
        assert!(
            child_count >= 2,
            "expanded usage should have at least 2 child nodes (off + on), got {}",
            child_count
        );
    }

    // ── Transition labels ────────────────────────────────────────────────

    #[test]
    fn transition_label_full() {
        let mut graph = ModelGraph::new();
        let state_def =
            Element::new_with_kind(ElementKind::StateDefinition).with_name("Machine");
        let def_id = graph.add_element(state_def);

        let s1 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("idle")
            .with_owner(def_id.clone());
        graph.add_element(s1);

        let s2 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("running")
            .with_owner(def_id.clone());
        graph.add_element(s2);

        let trans = Element::new_with_kind(ElementKind::TransitionUsage)
            .with_owner(def_id)
            .with_prop("source", "idle")
            .with_prop("target", "running");
        let trans_id = graph.add_element(trans);
        graph.add_transition_feature(
            &trans_id,
            "trigger",
            Element::new_with_kind(ElementKind::AcceptActionUsage).with_prop("text", "startEvent"),
        );
        graph.add_transition_feature(
            &trans_id,
            "guard",
            Element::new_with_kind(ElementKind::Expression).with_prop("text", "isReady"),
        );
        graph.add_transition_feature(
            &trans_id,
            "effect",
            Element::new_with_kind(ElementKind::ActionUsage).with_prop("text", "logStart"),
        );

        let gen = StateTransitionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &HashSet::new()));

        // Find the labeled transition edge (not the initial→first edge)
        let edges = all_edges(&ir);
        let trans_edge = edges
            .iter()
            .find(|e| matches!(&e.kind, DiagramEdgeKind::Transition { trigger, .. } if trigger.is_some()))
            .expect("should have transition edge with trigger");

        match &trans_edge.kind {
            DiagramEdgeKind::Transition { trigger, guard } => {
                assert_eq!(trigger.as_deref(), Some("startEvent"));
                assert_eq!(guard.as_deref(), Some("isReady"));
            }
            _ => panic!("expected Transition edge kind"),
        }
        assert!(
            trans_edge.label.contains("logStart"),
            "expected effect in label, got: {}",
            trans_edge.label
        );
    }

    #[test]
    fn transition_label_trigger_only() {
        let mut graph = ModelGraph::new();
        let state_def =
            Element::new_with_kind(ElementKind::StateDefinition).with_name("Machine");
        let def_id = graph.add_element(state_def);

        let s1 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("on")
            .with_owner(def_id.clone());
        graph.add_element(s1);

        let s2 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("off")
            .with_owner(def_id.clone());
        graph.add_element(s2);

        let trans = Element::new_with_kind(ElementKind::TransitionUsage)
            .with_owner(def_id)
            .with_prop("source", "on")
            .with_prop("target", "off");
        let trans_id = graph.add_element(trans);
        graph.add_transition_feature(
            &trans_id,
            "trigger",
            Element::new_with_kind(ElementKind::AcceptActionUsage).with_prop("text", "powerOff"),
        );

        let gen = StateTransitionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &HashSet::new()));

        let edges = all_edges(&ir);
        let trans_edge = edges
            .iter()
            .find(|e| matches!(&e.kind, DiagramEdgeKind::Transition { trigger, .. } if trigger.is_some()))
            .expect("should have transition edge with trigger");

        match &trans_edge.kind {
            DiagramEdgeKind::Transition { trigger, guard } => {
                assert_eq!(trigger.as_deref(), Some("powerOff"));
                assert!(guard.is_none(), "should not have guard");
            }
            _ => panic!("expected Transition edge kind"),
        }
    }

    // ── CSS classes ──────────────────────────────────────────────────────

    #[test]
    fn ref_css_class_for_typed_usage() {
        let mut graph = ModelGraph::new();
        let state_def =
            Element::new_with_kind(ElementKind::StateDefinition).with_name("ExternalDef");
        graph.add_element(state_def);

        let usage = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("myRef")
            .with_prop("unresolved_type", "ExternalDef");
        graph.add_element(usage);

        let gen = StateTransitionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &HashSet::new()));

        let ref_node = ir
            .nodes
            .iter()
            .find(|n| n.name.contains("myRef"))
            .expect("should find myRef");
        assert!(
            ref_node.tags.contains(&NodeTag::SubmachineRef),
            "typed usage should have SubmachineRef tag"
        );
    }

    #[test]
    fn exhibit_state_css_class_and_keyword() {
        let mut graph = ModelGraph::new();
        let state_def =
            Element::new_with_kind(ElementKind::StateDefinition).with_name("Container");
        let def_id = graph.add_element(state_def);

        let exhibit = Element::new_with_kind(ElementKind::ExhibitStateUsage)
            .with_name("displayed")
            .with_owner(def_id);
        graph.add_element(exhibit);

        let gen = StateTransitionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &HashSet::new()));

        let container = &ir.nodes[0];
        let exhibit_node = container
            .children
            .iter()
            .find_map(|c| match c {
                DiagramChild::Node(n) if n.name.contains("displayed") => Some(n),
                _ => None,
            })
            .expect("should find 'displayed' node");

        assert!(
            exhibit_node.tags.contains(&NodeTag::ExhibitState),
            "exhibit state should have ExhibitState tag"
        );
        assert!(
            exhibit_node.name.contains("\u{00AB}exhibit-state\u{00BB}"),
            "exhibit state name should have keyword prefix, got: {}",
            exhibit_node.name
        );
    }

    #[test]
    fn parallel_badge() {
        let mut graph = ModelGraph::new();
        let state_def =
            Element::new_with_kind(ElementKind::StateDefinition).with_name("Container");
        let def_id = graph.add_element(state_def);

        let state = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("concurrent")
            .with_prop("isParallel", true)
            .with_owner(def_id);
        graph.add_element(state);

        let gen = StateTransitionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &HashSet::new()));

        let container = &ir.nodes[0];
        let parallel_node = container
            .children
            .iter()
            .find_map(|c| match c {
                DiagramChild::Node(n) if n.name.contains("concurrent") => Some(n),
                _ => None,
            })
            .expect("should find 'concurrent' node");

        assert!(
            parallel_node.name.contains("\u{00AB}parallel\u{00BB}"),
            "parallel state should have badge, got: {}",
            parallel_node.name
        );
    }

    // ── Dedup ────────────────────────────────────────────────────────────

    #[test]
    fn dedup_transition_usage_and_relationship() {
        let mut graph = ModelGraph::new();
        let def =
            Element::new_with_kind(ElementKind::StateDefinition).with_name("Machine");
        let def_id = graph.add_element(def);

        let s_off = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("off")
            .with_owner(def_id.clone());
        let off_id = graph.add_element(s_off);

        let s_on = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("on")
            .with_owner(def_id.clone());
        let on_id = graph.add_element(s_on);

        // Path 1: TransitionUsage element
        let trans = Element::new_with_kind(ElementKind::TransitionUsage)
            .with_owner(def_id.clone())
            .with_prop("source", "off")
            .with_prop("target", "on");
        let trans_id = graph.add_element(trans);
        graph.add_transition_feature(
            &trans_id,
            "trigger",
            Element::new_with_kind(ElementKind::AcceptActionUsage).with_prop("text", "powerOn"),
        );

        // Path 2: Elaborated Relationship
        let rel = Relationship::new(RelationshipKind::Transition, off_id, on_id)
            .with_prop("event", "powerOn");
        graph.add_relationship(rel);

        let gen = StateTransitionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &HashSet::new()));

        // Count transition edges (not initial edges)
        let transition_count = ir
            .edges
            .iter()
            .filter(|e| {
                matches!(&e.kind, DiagramEdgeKind::Transition { trigger, .. } if trigger.is_some())
            })
            .count();
        assert!(
            transition_count <= 1,
            "dedup should prevent duplicate transition, got {}",
            transition_count
        );
    }

    #[test]
    fn distinct_transitions_not_deduped() {
        let mut graph = ModelGraph::new();
        let def =
            Element::new_with_kind(ElementKind::StateDefinition).with_name("Machine");
        let def_id = graph.add_element(def);

        let s_a = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("a")
            .with_owner(def_id.clone());
        graph.add_element(s_a);

        let s_b = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("b")
            .with_owner(def_id.clone());
        graph.add_element(s_b);

        for trigger in ["start", "restart"] {
            let trans = Element::new_with_kind(ElementKind::TransitionUsage)
                .with_owner(def_id.clone())
                .with_prop("source", "a")
                .with_prop("target", "b");
            let trans_id = graph.add_element(trans);
            graph.add_transition_feature(
                &trans_id,
                "trigger",
                Element::new_with_kind(ElementKind::AcceptActionUsage).with_prop("text", trigger),
            );
        }

        let gen = StateTransitionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &HashSet::new()));

        let edges = all_edges(&ir);
        let triggers: Vec<_> = edges
            .iter()
            .filter_map(|e| match &e.kind {
                DiagramEdgeKind::Transition { trigger, .. } => trigger.clone(),
                _ => None,
            })
            .collect();

        assert!(triggers.contains(&"start".to_string()), "first trigger should exist");
        assert!(
            triggers.contains(&"restart".to_string()),
            "second trigger should exist"
        );
    }

    // ── generate_for_owner ───────────────────────────────────────────────

    #[test]
    fn generate_for_owner_only_includes_owned_states() {
        let mut graph = ModelGraph::new();

        let def_a =
            Element::new_with_kind(ElementKind::StateDefinition).with_name("MachineA");
        let def_a_id = graph.add_element(def_a);

        let s1 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Idle")
            .with_owner(def_a_id.clone());
        graph.add_element(s1);

        let s2 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Running")
            .with_owner(def_a_id.clone());
        graph.add_element(s2);

        let def_b =
            Element::new_with_kind(ElementKind::StateDefinition).with_name("MachineB");
        let def_b_id = graph.add_element(def_b);

        let s3 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Off")
            .with_owner(def_b_id.clone());
        graph.add_element(s3);

        let gen = StateTransitionViewGenerator;
        let ir_a = gen
            .generate_for_owner(
                &make_ctx(&graph, &HashSet::new()),
                &def_a_id.to_string(),
            )
            .expect("should produce IR for owner A");

        let names_a: Vec<_> = ir_a.nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(
            names_a.iter().any(|n| n.contains("Idle")),
            "A subtree should contain Idle"
        );
        assert!(
            names_a.iter().any(|n| n.contains("Running")),
            "A subtree should contain Running"
        );
        assert!(
            !names_a.iter().any(|n| n.contains("Off")),
            "A subtree should NOT contain Off from B"
        );
    }

    #[test]
    fn generate_for_owner_empty_when_no_children() {
        let mut graph = ModelGraph::new();
        let def =
            Element::new_with_kind(ElementKind::StateDefinition).with_name("Empty");
        let def_id = graph.add_element(def);

        let gen = StateTransitionViewGenerator;
        let ir = gen
            .generate_for_owner(
                &make_ctx(&graph, &HashSet::new()),
                &def_id.to_string(),
            )
            .expect("should produce IR");

        assert!(ir.nodes.is_empty(), "should be empty when no state children");
    }

    #[test]
    fn generate_for_owner_includes_transitions() {
        let mut graph = ModelGraph::new();
        let def =
            Element::new_with_kind(ElementKind::StateDefinition).with_name("Machine");
        let def_id = graph.add_element(def);

        let s1 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("off")
            .with_owner(def_id.clone());
        graph.add_element(s1);

        let s2 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("on")
            .with_owner(def_id.clone());
        graph.add_element(s2);

        let trans = Element::new_with_kind(ElementKind::TransitionUsage)
            .with_owner(def_id.clone())
            .with_prop("source", "off")
            .with_prop("target", "on");
        let trans_id = graph.add_element(trans);
        graph.add_transition_feature(
            &trans_id,
            "trigger",
            Element::new_with_kind(ElementKind::AcceptActionUsage).with_prop("text", "powerOn"),
        );

        let gen = StateTransitionViewGenerator;
        let ir = gen
            .generate_for_owner(
                &make_ctx(&graph, &HashSet::new()),
                &def_id.to_string(),
            )
            .expect("should produce IR");

        assert!(
            !ir.edges.is_empty(),
            "should have transition edges"
        );
        let has_trigger = ir.edges.iter().any(|e| {
            matches!(&e.kind, DiagramEdgeKind::Transition { trigger, .. } if trigger.as_deref() == Some("powerOn"))
        });
        assert!(has_trigger, "should have transition with trigger 'powerOn'");
    }

    #[test]
    fn generate_for_owner_includes_relationship_transitions() {
        let mut graph = ModelGraph::new();
        let def =
            Element::new_with_kind(ElementKind::StateDefinition).with_name("Machine");
        let def_id = graph.add_element(def);

        let s1 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Idle")
            .with_owner(def_id.clone());
        let s1_id = graph.add_element(s1);

        let s2 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Running")
            .with_owner(def_id.clone());
        let s2_id = graph.add_element(s2);

        let transition =
            Relationship::new(RelationshipKind::Transition, s1_id, s2_id);
        graph.add_relationship(transition);

        let gen = StateTransitionViewGenerator;
        let ir = gen
            .generate_for_owner(
                &make_ctx(&graph, &HashSet::new()),
                &def_id.to_string(),
            )
            .expect("should produce IR");

        let trans_edges: Vec<_> = ir
            .edges
            .iter()
            .filter(|e| matches!(&e.kind, DiagramEdgeKind::Transition { .. }))
            .collect();
        assert!(
            !trans_edges.is_empty(),
            "should have relationship-based transition edges"
        );
    }

    // ── Nested state text labels ─────────────────────────────────────────

    #[test]
    fn collapsed_node_with_nested_states_shows_text_labels() {
        let mut graph = ModelGraph::new();

        let parent = Element::new_with_kind(ElementKind::StateUsage).with_name("parent");
        let parent_id = graph.add_element(parent);

        let child = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("child1")
            .with_owner(parent_id);
        graph.add_element(child);

        let gen = StateTransitionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &HashSet::new()));

        let parent_node = ir
            .nodes
            .iter()
            .find(|n| n.name.contains("parent"))
            .expect("should find parent");

        let text_children: Vec<_> = parent_node
            .children
            .iter()
            .filter_map(|c| match c {
                DiagramChild::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            text_children.iter().any(|t| t.contains("child1")),
            "collapsed parent should show nested state as text, got {:?}",
            text_children
        );
    }

    // (removed `source_location_propagates` — node source was dropped in 3.15;
    //  source spans now live only in the ViewModel text-map.)

    // ── Depth limiting ───────────────────────────────────────────────────

    #[test]
    fn max_depth_produces_leaf_node() {
        let element = Element::new_with_kind(ElementKind::StateUsage).with_name("deep");
        let graph = ModelGraph::new();
        let expanded = HashSet::new();

        // Call at MAX_STATE_DEPTH to trigger the guard
        let mut ir = DiagramIR::new(ViewType::StateTransition);
        let node = generate_state_node_inner(&graph, &element, &expanded, MAX_STATE_DEPTH, &mut ir);
        assert!(
            node.name.contains("max depth"),
            "max depth node should indicate truncation, got: {}",
            node.name
        );
    }

    // ── Composite (nested) states ─────────────────────────────────────────

    #[test]
    fn nested_states_not_rendered_as_siblings() {
        // brewing is a StateUsage with sub-states (preInfusion, extraction, draining).
        // The sub-states must NOT appear as top-level siblings of brewing.
        let mut graph = ModelGraph::new();

        let state_def = Element::new_with_kind(ElementKind::StateDefinition)
            .with_name("BrewController");
        let def_id = graph.add_element(state_def);

        let standby = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("standby")
            .with_owner(def_id.clone());
        let _standby_id = graph.add_element(standby);

        let brewing = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("brewing")
            .with_owner(def_id.clone());
        let brewing_id = graph.add_element(brewing);

        // Sub-states owned by brewing (a StateUsage, not a StateDefinition)
        let pre = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("preInfusion")
            .with_owner(brewing_id.clone());
        let _pre_id = graph.add_element(pre);

        let ext = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("extraction")
            .with_owner(brewing_id.clone());
        let _ext_id = graph.add_element(ext);

        let drain = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("draining")
            .with_owner(brewing_id.clone());
        let _drain_id = graph.add_element(drain);

        let gen = StateTransitionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &HashSet::new()));

        // Only BrewController container should be a top-level node
        let top_level_names: Vec<_> = ir.nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(
            !top_level_names.contains(&"preInfusion"),
            "preInfusion should NOT be a top-level node, got: {:?}",
            top_level_names
        );
        assert!(
            !top_level_names.contains(&"extraction"),
            "extraction should NOT be a top-level node, got: {:?}",
            top_level_names
        );
        assert!(
            !top_level_names.contains(&"draining"),
            "draining should NOT be a top-level node, got: {:?}",
            top_level_names
        );
    }

    #[test]
    fn expanded_composite_state_has_edges() {
        // When a composite state (brewing) is expanded, the edges between its
        // sub-states must be generated.
        let mut graph = ModelGraph::new();

        let state_def = Element::new_with_kind(ElementKind::StateDefinition)
            .with_name("BrewController");
        let def_id = graph.add_element(state_def);

        let brewing = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("brewing")
            .with_owner(def_id.clone());
        let brewing_id = graph.add_element(brewing);

        let pre = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("preInfusion")
            .with_owner(brewing_id.clone());
        let pre_id = graph.add_element(pre);

        let ext = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("extraction")
            .with_owner(brewing_id.clone());
        let ext_id = graph.add_element(ext);

        // Add a transition between sub-states
        let mut trans = Element::new_with_kind(ElementKind::TransitionUsage)
            .with_name("pre_to_ext")
            .with_owner(brewing_id.clone());
        trans.set_prop("source", sysml_core::Value::String("preInfusion".into()));
        trans.set_prop("target", sysml_core::Value::String("extraction".into()));
        let trans_id = graph.add_element(trans);
        graph.add_transition_feature(
            &trans_id,
            "trigger",
            Element::new_with_kind(ElementKind::AcceptActionUsage)
                .with_prop("text", "preInfusionComplete"),
        );

        // Expand brewing
        let mut expanded = HashSet::new();
        expanded.insert(brewing_id.to_string());

        let gen = StateTransitionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &expanded));

        // Should have transition edges for the sub-states (nested inside container)
        let edges = all_edges(&ir);
        let sub_edges: Vec<_> = edges
            .iter()
            .filter(|e| {
                e.source_id == pre_id.to_string() && e.target_id == ext_id.to_string()
            })
            .collect();
        assert!(
            !sub_edges.is_empty(),
            "expected transition edge between preInfusion and extraction inside expanded brewing"
        );

        // Should also have initial→first-child edge inside expanded brewing
        let init_edges: Vec<_> = edges
            .iter()
            .filter(|e| e.source_id.contains("initial"))
            .collect();
        assert!(
            !init_edges.is_empty(),
            "expected initial edge inside expanded brewing"
        );
    }
}
